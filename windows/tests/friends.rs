// SPDX-License-Identifier: MIT
//! フレンド一覧の取り直しと、テスト通知。API は呼ばない（取り直しは偽の fetcher）。
use std::path::Path;
use std::time::Duration;
use vwn::config::Config;
use vwn::watcher::{classify_fetch, FetchResult, Out, Watcher};

fn config(dir: &Path) -> Config {
    let p = dir.join("config.json");
    std::fs::write(&p, serde_json::json!({ "log_dir": dir, "friends_file": dir.join("friends.json") }).to_string()).unwrap();
    Config::load(&p).0
}

fn live(dir: &Path) -> (Watcher, std::sync::mpsc::Receiver<Out>) {
    let (tx, rx) = std::sync::mpsc::channel();
    (Watcher::new(config(dir), Some(tx)), rx)
}

fn ok_now(_: String) -> FetchResult {
    FetchResult { ok: true, ids: vec!["usr_new".into()], code: "200".into(), ..Default::default() }
}
fn ok_slow(_: String) -> FetchResult {
    std::thread::sleep(Duration::from_millis(300));
    FetchResult { ok: true, ids: vec!["usr_old".into(), "usr_old2".into()], code: "200".into(), ..Default::default() }
}
fn unauthorized_slow(_: String) -> FetchResult {
    std::thread::sleep(Duration::from_millis(300));
    FetchResult { code: "401".into(), ..Default::default() }
}

fn settle(w: &mut Watcher) {
    std::thread::sleep(Duration::from_millis(500));
    w.poll_friends_refresh(false);
}

#[test]
fn clearing_mid_refresh_is_not_overwritten() {
    let d = tempfile::tempdir().unwrap();
    let (mut w, _rx) = live(d.path());
    w.fetcher = Some(ok_slow);
    w.manual_refresh();
    assert!(w.status().refreshing);
    w.key_cleared();
    assert!(!w.status().refreshing);
    settle(&mut w);
    assert_eq!(w.status().friends, "nokey");
    assert_eq!(w.status().friends_count, 0);
}

#[test]
fn saving_mid_refresh_uses_the_new_key() {
    let d = tempfile::tempdir().unwrap();
    let (mut w, _rx) = live(d.path());
    // 古いキーでの取り直しが遅れて 401 を返しても、新しいキーでの結果が勝つ
    w.fetcher = Some(unauthorized_slow);
    w.manual_refresh();
    w.fetcher = Some(ok_now);
    w.manual_refresh();
    w.poll_friends_refresh(true);
    assert_eq!(w.status().friends, "ok");
    assert_eq!(w.status().friends_count, 1);
    settle(&mut w);
    assert_eq!(w.status().friends, "ok");
}

#[test]
fn network_errors_are_not_key_errors() {
    assert_eq!(classify_fetch("nokey"), ("nokey", true));
    assert_eq!(classify_fetch("unreadable"), ("unreadable", true));
    assert_eq!(classify_fetch("401"), ("bad", true));
    assert_eq!(classify_fetch("nofriends"), ("bad", true));
    for c in ["net", "500", "502", "503", "429", "error"] {
        assert_eq!(classify_fetch(c), ("neterror", false), "{c}");
    }
}

#[test]
fn send_test_is_its_own_kind() {
    let d = tempfile::tempdir().unwrap();
    let mut w = Watcher::new(config(d.path()), None);
    w.send_test();
    let out = w.take_output();
    assert_eq!(out.len(), 1);
    assert!(out[0].starts_with(&format!("{}\t", vwn::toast::TEST_KIND)), "{out:?}");
    assert!(!out[0].starts_with("join\t"));
    assert_eq!(vwn::toast::TEST_KIND, "info.test");
    for k in ["info", "info.other", "care.test", "join", "alert.test", "info.test2"] {
        assert!(!vwn::toast::is_test(k), "{k}");
    }
    assert!(vwn::toast::is_test("info.test"));
}
