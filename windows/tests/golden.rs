// SPDX-License-Identifier: MIT
//! 0.4 までの tests/run-windows-tests.ps1 を移したもの。合成ログを --replay / --backfill / --show-world の
//! dry-run で読み、検出結果が tests/expected_*.tsv と 1 行ずつ同じか確かめる。API は呼ばない。
//!   cd windows && cargo test
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use vwn::config::Config;
use vwn::watcher::Watcher;

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

fn tmpdir(tag: &str) -> PathBuf {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("vrc-wrist-notifier-test-{tag}-{n}"));
    fs::create_dir_all(&d).unwrap();
    d
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
    let mut w = Watcher::new(cfg, true);
    w.replay(&here().join(log)).unwrap();
    w.take_output()
}

#[test]
fn replay_events() {
    let d = tmpdir("events");
    check(replay(config(&d, serde_json::json!({})), "sample_output_log.txt"), "expected_events.tsv");
}

#[test]
fn replay_batch() {
    // 参加・退出のまとめ（1 人、2 人、3 人、大勢、参加と退出が混ざる、ワールドが違う）
    let d = tmpdir("batch");
    check(replay(config(&d, serde_json::json!({})), "sample_batch_log.txt"), "expected_batch.tsv");
}

/// 過去のログからの履歴: 2 つ目のファイルは、ワールドの行より前の参加（前のファイルのワールドを引き継がない）、絵文字のワールド名、ワールド ID
fn two_logs() -> PathBuf {
    let d = tmpdir("history");
    place(&d, "sample_output_log.txt", "output_log_2026-01-02_21-00-00.txt", 1_767_355_200);
    place(&d, "sample_world_log.txt", "output_log_2026-01-03_20-00-00.txt", 1_767_441_600);
    d
}

#[test]
fn backfill_history() {
    let d = two_logs();
    let mut w = Watcher::new(config(&d, serde_json::json!({})), true);
    w.backfill();
    check(w.take_output(), "expected_history.tsv");
}

#[test]
fn show_world() {
    // 今いるワールド: いちばん新しいログの最後のワールド
    let d = two_logs();
    let mut w = Watcher::new(config(&d, serde_json::json!({})), true);
    let newest = w.newest_log().unwrap();
    w.read_current_world(&newest);
    check(vec![w.world_line()], "expected_world.tsv");
}

#[test]
fn friends() {
    // friends.json の人は印が付き、まとめカードの先頭にはフレンドが来る
    let d = tmpdir("friend");
    place(&d, "sample_friend_log.txt", "output_log_2026-01-06_20-00-00.txt", 1_767_700_000);
    let extra = serde_json::json!({ "friends_file": here().join("sample_friends.json") });
    check(replay(config(&d, extra.clone()), "sample_friend_log.txt"), "expected_friend.tsv");
    let mut w = Watcher::new(config(&d, extra), true);
    w.backfill();
    check(w.take_output(), "expected_friend_history.tsv");
}

#[test]
fn status_line() {
    let d = two_logs();
    let mut w = Watcher::new(config(&d, serde_json::json!({})), true);
    assert_eq!(w.status_line(), "log\tfull\toutput_log_2026-01-03_20-00-00.txt");
    let e = tmpdir("nolog");
    let mut w = Watcher::new(config(&e, serde_json::json!({})), true);
    assert_eq!(w.status_line(), "log\tnolog");
}

#[test]
fn osc_wire_format() {
    // PowerShell 版と同じバイト列（文字列は NUL で 4 バイト境界まで埋める。ちょうどでも 1 つ以上）
    let p = vwn::watcher::osc_packet("/wrist/heartbeat", &["0.5.0", "full", "ok"]);
    let mut want = b"/wrist/heartbeat\0\0\0\0,sss\0\0\0\x000.5.0\0\0\0full\0\0\0\0ok\0\0".to_vec();
    assert_eq!(p, std::mem::take(&mut want));
}

#[test]
fn config_defaults_and_unknown_keys() {
    let d = tmpdir("cfg");
    let p = d.join("config.json");
    fs::write(&p, "\u{feff}{\"_comment\":\"x\",\"frame_host\":\"192.168.0.9\",\"notify_leave\":false,\"bogus\":1}").unwrap();
    let (c, warn) = Config::load(&p);
    assert_eq!(c.frame_host, "192.168.0.9");
    assert_eq!(c.frame_port, 39570);
    assert!(!c.notify_leave && c.notify_join && !c.notify_other && c.friends);
    assert_eq!(c.batch_seconds, 3.0);
    assert_eq!(c.friends_refresh_minutes, 60.0);
    assert_eq!(warn, vec!["知らない設定項目 bogus は無視します".to_string()]);
}

#[test]
fn self_leave_teardown() {
    // 自分がワールドを出たとき（OnLeftRoom → 全員分の OnPlayerLeft → 自分 → Entering Room）や VRChat を終えたときの
    // 他人の退出は出さない（通知も履歴も）。自分がいるあいだの他人の退出（Ann, 2 つ目のワールドの Bob）はそのまま
    let d = tmpdir("teardown");
    place(&d, "sample_teardown_log.txt", "output_log_2026-01-07_21-00-00.txt", 1_767_800_000);
    check(replay(config(&d, serde_json::json!({})), "sample_teardown_log.txt"), "expected_teardown.tsv");
    let mut w = Watcher::new(config(&d, serde_json::json!({})), true);
    w.backfill();
    check(w.take_output(), "expected_teardown_history.tsv");
}
