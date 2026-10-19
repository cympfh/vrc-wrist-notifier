// 描画のテスト: カードとパネルが描け、PNG に書けること（フォントが無い環境では形だけ）
use std::collections::VecDeque;
use vwn::history::HistEvent;
use vwn::panel::{HistoryPanel, PanelStatus};
use vwn::toast::{self, Notify, Toast};

fn ev(t: &str, k: &str, n: &str, w: &str, fr: bool, me: bool) -> HistEvent {
    HistEvent { t: t.into(), kind: k.into(), name: n.into(), world: w.into(), key: format!("{t}|{k}|{n}"), fr, me, ..Default::default() }
}

pub fn sample() -> VecDeque<HistEvent> {
    let mut v = VecDeque::new();
    v.push_back(ev("2026-10-18 21:00:00", "world", "The Black Cat", "The Black Cat", false, false));
    v.push_back(ev("2026-10-18 21:00:05", "join", "Me", "The Black Cat", false, true));
    v.push_back(ev("2026-10-18 21:03:10", "join", "Alice", "The Black Cat", true, false));
    v.push_back(ev("2026-10-18 21:05:00", "join", "Bob", "The Black Cat", false, false));
    v.push_back(ev("2026-10-18 21:40:00", "leave", "Bob", "The Black Cat", false, false));
    v.push_back(ev("2026-10-19 00:10:00", "world", "Midnight Rooftop", "Midnight Rooftop", false, false));
    v.push_back(ev("2026-10-19 00:10:04", "join", "Me", "Midnight Rooftop", false, true));
    v.push_back(ev("2026-10-19 00:12:00", "join", "Carol", "Midnight Rooftop", true, false));
    v
}

/// PNG を書く先（VWN_PNG_DIR があればそこに残す。無ければ一時フォルダで、終わると消える）
fn out(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    match std::env::var_os("VWN_PNG_DIR") {
        Some(d) => std::path::PathBuf::from(d).join(name),
        None => dir.path().join(name),
    }
}

#[test]
fn toast_renders() {
    let tmp = tempfile::tempdir().unwrap();
    let mut t = Toast::new("join", "Alice さんが参加しました", "The Black Cat");
    t.fr = true;
    let c = toast::render(&t);
    assert_eq!(c.to_rgba().len(), (toast::W * toast::H * 4) as usize);
    c.save_png(&out(&tmp, "vwn-test-toast.png")).unwrap();
}

#[test]
fn panel_renders_and_switches_notify() {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = HistoryPanel::new();
    p.set_status(&PanelStatus { log: "full".into(), friends: "nokey".into() });
    let c = p.render(&sample());
    c.save_png(&out(&tmp, "vwn-test-panel.png")).unwrap();
    // 「フレンドのみ」を押す
    assert!(!p.take_notify_change());
    p.pointer_down(190.0 + 300.0, 112.0 + 12.0 + 26.0);
    p.pointer_up(190.0 + 300.0, 112.0 + 12.0 + 26.0);
    assert_eq!(p.notify(), Notify::Friends);
    assert!(p.take_notify_change());
    assert!(!p.take_notify_change());
    HistoryPanel::new().render(&VecDeque::new()).save_png(&out(&tmp, "vwn-test-empty.png")).unwrap();
}

#[test]
fn corner_note_only_for_problems() {
    use vwn::panel::corner_note;
    let st = |l: &str, f: &str| PanelStatus { log: l.into(), friends: f.into() };
    assert_eq!(corner_note(&st("full", "ok")), None);
    assert_eq!(corner_note(&st("full", "off")), None);
    assert_eq!(corner_note(&st("full", "nokey")), Some(vwn::auth::PANEL_NOKEY));
    assert_eq!(corner_note(&st("full", "bad")), Some(vwn::auth::PANEL_BAD));
    assert!(corner_note(&st("errors", "ok")).unwrap().contains("Full"));
}

#[test]
fn friends_only_and_off_filter() {
    let t = Toast::new("join", "Bob さんほか 3 人", "が参加");
    let fr = vec!["Alice".to_string()];
    assert!(toast::filter_people(&t, Notify::Off, &fr, "").is_empty());
    assert!(toast::filter_people(&Toast::new("invite", "A", "から招待"), Notify::Off, &[], "").is_empty());
    assert_eq!(toast::filter_people(&t, Notify::All, &fr, "").len(), 1);
    let only = toast::filter_people(&t, Notify::Friends, &fr, "Me");
    assert_eq!(only.len(), 2);
    assert!(only[0].me && only[0].title == "Me" && only[1].fr && only[1].title == "Alice");
    assert!(toast::filter_people(&t, Notify::Friends, &[], "").is_empty());
}

#[test]
fn history_jsonl_roundtrip_same_format_as_frame() {
    // 0.5 までの Frame の history.jsonl の 1 行をそのまま読めて、書き戻すと同じ形
    let line = r#"{"t":"2026-10-18 21:03:10","k":"join","n":"Alice","w":"The Black Cat","key":"2026-10-18 21:03:10|join|Alice","wid":"wrld_1","f":"1"}"#;
    let e = HistEvent::from_json(line).unwrap();
    assert!(e.fr && !e.me && e.wid == "wrld_1");
    assert_eq!(HistEvent::from_json(&e.to_json()).unwrap(), e);
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().to_path_buf();
    let mut h = vwn::history::History::new(d.clone());
    for e in sample() {
        h.add(e);
    }
    assert!(!h.add(sample()[1].clone())); // 同じ鍵は 1 回だけ
    let mut h2 = vwn::history::History::new(d.clone());
    h2.load();
    assert_eq!(h2.events().len(), sample().len());
}

// ---------------------------------------------------------------- 途中で切れた履歴（PC が落ちた・書いている途中で止まった）
fn line(t: &str, n: &str) -> String {
    HistEvent { t: t.into(), kind: "join".into(), name: n.into(), world: "W".into(), key: format!("{t}|join|{n}"), ..Default::default() }.to_json()
}

#[test]
fn truncated_multibyte_last_line_keeps_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("history.jsonl");
    let mut bytes = format!("{}\n{}\n", line("2026-10-19 10:00:00", "Ann"), line("2026-10-19 10:01:00", "Bob")).into_bytes();
    let cut = line("2026-10-19 10:02:00", "ペンちゃん");
    let i = cut.find("ちゃ").unwrap() + 1; // 「ち」の途中で切る
    bytes.extend_from_slice(&cut.as_bytes()[..i]);
    std::fs::write(&p, &bytes).unwrap();
    let mut h = vwn::history::History::new(tmp.path().to_path_buf());
    h.load();
    assert_eq!(h.events().len(), 2);
    // 読み込み直し（backfill）で同じものを足しても増えない
    assert!(!h.add(HistEvent::from_json(&line("2026-10-19 10:00:00", "Ann")).unwrap()));
}

#[test]
fn truncated_ascii_line_then_append() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("history.jsonl");
    let cut = line("2026-10-19 10:01:00", "Bob");
    std::fs::write(&p, format!("{}\n{}", line("2026-10-19 10:00:00", "Ann"), &cut[..cut.len() / 2])).unwrap();
    let mut h = vwn::history::History::new(tmp.path().to_path_buf());
    h.load();
    assert_eq!(h.events().len(), 1);
    assert!(h.add(HistEvent::from_json(&line("2026-10-19 10:05:00", "Cat")).unwrap()));
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.ends_with('\n') && text.lines().last().unwrap().contains("Cat"), "{text}");
    let mut h2 = vwn::history::History::new(tmp.path().to_path_buf());
    h2.load();
    let names: Vec<_> = h2.events().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Ann", "Cat"]);
}

#[test]
fn concurrent_appends_do_not_interleave() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let threads: Vec<_> = (0..4)
        .map(|k| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let mut h = vwn::history::History::new(dir);
                for i in 0..200 {
                    h.add(HistEvent::from_json(&line(&format!("2026-10-19 10:{:02}:{:02}", k, i % 60), &format!("名前{k}-{i}"))).unwrap());
                }
            })
        })
        .collect();
    threads.into_iter().for_each(|t| t.join().unwrap());
    let text = std::fs::read_to_string(dir.join("history.jsonl")).unwrap();
    // 書いている最中の行を読んだ別の書き手が改行を足すことがあり、空行はありうる（読むときは飛ばす）。中身の行は混ざらない
    let lines: Vec<_> = text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 800);
    assert!(lines.iter().all(|l| HistEvent::from_json(l).is_some()));
    let mut h = vwn::history::History::new(dir.clone());
    h.load();
    assert_eq!(h.events().len(), 800);
}

#[test]
fn test_toast_ignores_notify_setting() {
    let t = Toast::new(toast::TEST_KIND, "テスト通知", "vrc-wrist-notifier から届きました");
    for mode in [Notify::All, Notify::Friends, Notify::Off] {
        let shown = toast::filter_people(&t, mode, &[], "");
        assert_eq!(shown.len(), 1, "{mode:?}");
        assert_eq!(shown[0].title, "テスト通知");
    }
    // ふつうの参加は今までどおり絞られる
    assert!(toast::filter_people(&Toast::new("join", "テスト", "が参加しました"), Notify::Friends, &[], "").is_empty());
    assert!(!toast::is_alert(toast::TEST_KIND));
}

#[test]
fn test_feedback_only_when_vr_is_down() {
    assert_eq!(toast::test_feedback(true), None);
    assert!(toast::test_feedback(false).unwrap().contains("SteamVR"));
}
