// SPDX-License-Identifier: MIT
//! 0.4 までの tests/run-windows-tests.ps1 を移したもの。合成ログを --replay / --backfill / --show-world の
//! dry-run で読み、検出結果が tests/expected_*.tsv と 1 行ずつ同じか確かめる。API は呼ばない。
//!   cd windows && cargo test
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use vwn::config::Config;
use vwn::hub::Hub;
use vwn::toast::Toast;
use vwn::watcher::{Out, Watcher};

fn here() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("tests")
}

fn want(name: &str) -> Vec<String> {
    fs::read_to_string(here().join(name)).unwrap().trim_start_matches('\u{feff}').lines().map(String::from).collect()
}

fn check(got: Vec<String>, name: &str) {
    let w = want(name);
    for i in 0..got.len().max(w.len()) {
        assert_eq!(got.get(i), w.get(i), "{name} の {} 行目", i + 1);
    }
}

/// テストごとの一時フォルダ（終わると消える。本物の %APPDATA% / %LOCALAPPDATA% には書かない）
fn tmpdir(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new().prefix(&format!("vwn-test-{tag}-")).tempdir().unwrap()
}

fn config(dir: &Path, extra: serde_json::Value) -> Config {
    let mut j = serde_json::json!({ "log_dir": dir, "friends_file": dir.join("no-friends.json") });
    for (k, v) in extra.as_object().unwrap() {
        j[k] = v.clone();
    }
    let p = dir.join("config.json");
    fs::write(&p, j.to_string()).unwrap();
    let (c, warn) = Config::load(&p);
    assert!(warn.is_empty(), "{warn:?}");
    c
}

fn place(dir: &Path, src: &str, name: &str, mtime_secs: u64) {
    let dst = dir.join(name);
    fs::copy(here().join(src), &dst).unwrap();
    let f = fs::File::options().write(true).open(&dst).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(mtime_secs)).unwrap();
}

fn replay(cfg: Config, log: &str) -> Vec<String> {
    let mut w = Watcher::new(cfg, None);
    w.replay(&here().join(log)).unwrap();
    w.take_output()
}

#[test]
fn replay_events() {
    let _td = tmpdir("events");
    let d = _td.path();
    check(replay(config(&d, serde_json::json!({})), "sample_output_log.txt"), "expected_events.tsv");
}

#[test]
fn replay_batch() {
    // 参加・退出のまとめ（1 人、2 人、3 人、大勢、参加と退出が混ざる、ワールドが違う）
    let _td = tmpdir("batch");
    let d = _td.path();
    check(replay(config(&d, serde_json::json!({})), "sample_batch_log.txt"), "expected_batch.tsv");
}

/// 過去のログからの履歴: 2 つ目のファイルは、ワールドの行より前の参加（前のファイルのワールドを引き継がない）、絵文字のワールド名、ワールド ID
fn two_logs() -> tempfile::TempDir {
    let td = tmpdir("history");
    let d = td.path();
    place(&d, "sample_output_log.txt", "output_log_2026-01-02_21-00-00.txt", 1_767_355_200);
    place(&d, "sample_world_log.txt", "output_log_2026-01-03_20-00-00.txt", 1_767_441_600);
    td
}

#[test]
fn backfill_history() {
    let td = two_logs();
    let d = td.path();
    let mut w = Watcher::new(config(&d, serde_json::json!({})), None);
    w.backfill();
    check(w.take_output(), "expected_history.tsv");
}

#[test]
fn show_world() {
    // 今いるワールド: いちばん新しいログの最後のワールド
    let td = two_logs();
    let d = td.path();
    let mut w = Watcher::new(config(&d, serde_json::json!({})), None);
    let newest = w.newest_log().unwrap();
    w.read_current_world(&newest);
    check(vec![w.world_line()], "expected_world.tsv");
}

#[test]
fn friends() {
    // friends.json の人は印が付き、まとめカードの先頭にはフレンドが来る
    let _td = tmpdir("friend");
    let d = _td.path();
    place(&d, "sample_friend_log.txt", "output_log_2026-01-06_20-00-00.txt", 1_767_700_000);
    let extra = serde_json::json!({ "friends_file": here().join("sample_friends.json") });
    check(replay(config(&d, extra.clone()), "sample_friend_log.txt"), "expected_friend.tsv");
    let mut w = Watcher::new(config(&d, extra), None);
    w.backfill();
    check(w.take_output(), "expected_friend_history.tsv");
}

#[test]
fn status_line() {
    let td = two_logs();
    let d = td.path();
    let mut w = Watcher::new(config(&d, serde_json::json!({})), None);
    assert_eq!(w.status_line(), "log\tfull\toutput_log_2026-01-03_20-00-00.txt");
    let _te = tmpdir("nolog");
    let e = _te.path();
    let mut w = Watcher::new(config(&e, serde_json::json!({})), None);
    assert_eq!(w.status_line(), "log\tnolog");
}

#[test]
fn live_sink_feeds_history_and_toasts() {
    // dry-run でないときは、通知と履歴が VR のスレッド向けのチャンネルに流れる
    let _td = tmpdir("live");
    let d = _td.path();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut w = Watcher::new(config(&d, serde_json::json!({})), Some(tx));
    w.replay(&here().join("sample_teardown_log.txt")).unwrap();
    drop(w);
    let (mut toasts, mut hist) = (0, Vec::new());
    for o in rx.try_iter() {
        match o {
            Out::Toast { .. } => toasts += 1,
            Out::History(e) => hist.push(e),
        }
    }
    assert!(toasts > 0);
    assert!(hist.iter().any(|e| e.kind == "world"));
    assert!(hist.iter().all(|e| !e.t.is_empty() && e.key == format!("{}|{}|{}", e.t, e.kind, e.name)));
}

#[test]
fn hub_notify_setting_saved_to_temp_config() {
    // 「通知設定」: オフなら何も出さない。パネルで選ぶと config.json のその項目だけ書き換わる（本物の %APPDATA% には触らない）
    let _td = tmpdir("hub");
    let d = _td.path();
    let cfg_path = d.join("config.json");
    fs::write(&cfg_path, "{\"_comment\":\"keep\",\"notify_people\":\"off\",\"notify_leave\":false}").unwrap();
    let (mut cfg, _) = Config::load(&cfg_path);
    cfg.history_dir = d.join("hist");
    let mut hub = Hub::new(&cfg, cfg_path.clone());
    let t = Out::Toast { toast: Toast::new("join", "Bob", "が参加しました"), friends: vec![], self_name: String::new() };
    hub.handle(t.clone());
    assert!(hub.queue.is_empty());
    // 「全員」を押す
    let (x, y) = (190.0 + 100.0, 112.0 + 12.0 + 26.0);
    hub.panel.pointer_down(x, y);
    hub.panel.pointer_up(x, y);
    hub.save_notify_if_changed();
    hub.handle(t);
    assert_eq!(hub.queue.len(), 1);
    let saved = fs::read_to_string(&cfg_path).unwrap();
    assert!(saved.contains("\"notify_people\": \"all\"") && saved.contains("keep") && saved.contains("\"notify_leave\": false"), "{saved}");
    assert!(saved.find("_comment").unwrap() < saved.find("notify_people").unwrap());
    // 履歴は temp の中だけ
    hub.handle(Out::History(vwn::history::HistEvent { t: "2026-10-19 12:00:00".into(), kind: "join".into(), name: "Bob".into(), key: "k".into(), ..Default::default() }));
    assert!(d.join("hist").join("history.jsonl").exists());
}

#[test]
fn config_defaults_and_unknown_keys() {
    let _td = tmpdir("cfg");
    let d = _td.path();
    let p = d.join("config.json");
    // 0.5 までの frame_host / frame_port は黙って読み飛ばす
    fs::write(&p, "\u{feff}{\"_comment\":\"x\",\"frame_host\":\"192.168.0.9\",\"frame_port\":1,\"notify_leave\":false,\"bogus\":1}").unwrap();
    let (c, warn) = Config::load(&p);
    assert_eq!(c.notify_people, "all");
    assert!(c.history_panel);
    assert!(!c.notify_leave && c.notify_join && !c.notify_other && c.friends);
    assert_eq!(c.batch_seconds, 3.0);
    assert_eq!(c.friends_refresh_minutes, 60.0);
    assert_eq!(warn, vec!["知らない設定項目 bogus は無視します".to_string()]);
}

#[test]
fn self_leave_teardown() {
    // 自分がワールドを出たとき（OnLeftRoom → 全員分の OnPlayerLeft → 自分 → Entering Room）や VRChat を終えたときの
    // 他人の退出は出さない（通知も履歴も）。自分がいるあいだの他人の退出（Ann, 2 つ目のワールドの Bob）はそのまま
    let _td = tmpdir("teardown");
    let d = _td.path();
    place(&d, "sample_teardown_log.txt", "output_log_2026-01-07_21-00-00.txt", 1_767_800_000);
    check(replay(config(&d, serde_json::json!({})), "sample_teardown_log.txt"), "expected_teardown.tsv");
    let mut w = Watcher::new(config(&d, serde_json::json!({})), None);
    w.backfill();
    check(w.take_output(), "expected_teardown_history.tsv");
}

// ---------------------------------------------------------------- 自分が出たときの片付け（0.5.1 のレビューの追加分）
const ME: &str = "usr_00000000-0000-0000-0000-000000000900";
fn ln(t: &str, body: &str) -> String {
    format!("2026.01.08 {t} Debug      -  {body}\n")
}
fn joined(t: &str, n: &str, id: u32) -> String {
    ln(t, &format!("[Behaviour] OnPlayerJoined {n} (usr_00000000-0000-0000-0000-000000000{id})"))
}
fn left(t: &str, n: &str, id: u32) -> String {
    ln(t, &format!("[Behaviour] OnPlayerLeft {n} (usr_00000000-0000-0000-0000-000000000{id})"))
}
fn history_leaves(out: &[String]) -> Vec<String> {
    out.iter().filter(|l| l.starts_with("history\t") && l.split('\t').nth(2) == Some("leave")).map(|l| l.split('\t').nth(3).unwrap().to_string()).collect()
}

#[test]
fn rejoin_without_entering_room_clears_teardown() {
    // OnLeftRoom のあと Entering Room 無しで自分がまた入った（OnPlayerJoined 自分）ら、その後の本当の退出は出す
    let _td = tmpdir("rejoin");
    let d = _td.path();
    let log = [
        ln("10:00:00", &format!("[Behaviour] User Authenticated: Me ({ME})")),
        ln("10:00:01", "[Behaviour] Entering Room: World A"),
        joined("10:00:02", "Me", 900),
        joined("10:01:00", "Bob", 902),
        ln("10:05:00", "[Behaviour] OnLeftRoom"),
        left("10:05:00", "Bob", 902),
        joined("10:05:10", "Me", 900),
        joined("10:06:00", "Bob", 902),
        left("10:20:00", "Bob", 902),
    ]
    .concat();
    fs::write(d.join("output_log_2026-01-08_10-00-00.txt"), &log).unwrap();
    let mut w = Watcher::new(config(&d, serde_json::json!({})), None);
    w.backfill();
    let out = w.take_output();
    assert_eq!(history_leaves(&out), vec!["Bob".to_string()], "{out:#?}");
    assert!(out.iter().any(|l| l.contains("2026-01-08 10:20:00\tleave\tBob")), "{out:#?}");
    // 通知（replay）でも同じ
    let mut w = Watcher::new(config(&d, serde_json::json!({})), None);
    w.replay(&d.join("output_log_2026-01-08_10-00-00.txt")).unwrap();
    let toasts: Vec<_> = w.take_output().into_iter().filter(|l| l.starts_with("leave\t")).collect();
    assert_eq!(toasts.len(), 1, "{toasts:?}");
}

#[test]
fn log_rotation_resets_teardown() {
    // 前のログが OnLeftRoom で終わっても（VRChat を終えた）、次のログファイルでは片付けの状態を引き継がない
    let _td = tmpdir("rotate");
    let d = _td.path();
    let a = [ln("09:00:00", &format!("[Behaviour] User Authenticated: Me ({ME})")), ln("09:00:01", "[Behaviour] Entering Room: World A"),
             joined("09:01:00", "Ann", 901), ln("09:30:00", "[Behaviour] OnLeftRoom"), left("09:30:00", "Ann", 901)].concat();
    let b = [joined("11:00:00", "Zed", 905), left("11:10:00", "Zed", 905)].concat();
    fs::write(d.join("output_log_2026-01-08_09-00-00.txt"), a).unwrap();
    fs::write(d.join("output_log_2026-01-08_11-00-00.txt"), b).unwrap();
    let f = |n: &str, s| fs::File::options().write(true).open(d.join(n)).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(s)).unwrap();
    f("output_log_2026-01-08_09-00-00.txt", 1_767_830_000);
    f("output_log_2026-01-08_11-00-00.txt", 1_767_840_000);
    let mut w = Watcher::new(config(&d, serde_json::json!({})), None);
    w.backfill();
    let out = w.take_output();
    assert_eq!(history_leaves(&out), vec!["Zed".to_string()], "{out:#?}");
}

#[test]
fn teardown_with_unknown_self_id() {
    // 自分の ID がわからない（User Authenticated の行が無い）ときも片付けの退出は出さない。自分の退出も見分けられないので出ない。
    // ワールドにいるあいだの退出はそのまま
    let _td = tmpdir("noself");
    let d = _td.path();
    let log = [
        ln("10:00:01", "[Behaviour] Entering Room: World A"),
        joined("10:00:02", "Me", 900),
        joined("10:01:00", "Ann", 901),
        joined("10:01:30", "Bob", 902),
        left("10:02:00", "Ann", 901),
        ln("10:05:00", "[Behaviour] OnLeftRoom"),
        left("10:05:00", "Bob", 902),
        left("10:05:00", "Me", 900),
        ln("10:05:02", "[Behaviour] Entering Room: World B"),
        joined("10:06:00", "Cat", 903),
        left("10:30:00", "Cat", 903),
    ]
    .concat();
    fs::write(d.join("output_log_2026-01-08_10-00-00.txt"), &log).unwrap();
    let mut w = Watcher::new(config(&d, serde_json::json!({})), None);
    w.backfill();
    let out = w.take_output();
    assert_eq!(history_leaves(&out), vec!["Ann".to_string(), "Cat".to_string()], "{out:#?}");
}
