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
    v.push_back(ev("2026-10-18 21:00:05", "join", "cympfh", "The Black Cat", false, true));
    v.push_back(ev("2026-10-18 21:03:10", "join", "Alice", "The Black Cat", true, false));
    v.push_back(ev("2026-10-18 21:05:00", "join", "Bob", "The Black Cat", false, false));
    v.push_back(ev("2026-10-18 21:40:00", "leave", "Bob", "The Black Cat", false, false));
    v.push_back(ev("2026-10-19 00:10:00", "world", "Midnight Rooftop", "Midnight Rooftop", false, false));
    v.push_back(ev("2026-10-19 00:10:04", "join", "cympfh", "Midnight Rooftop", false, true));
    v.push_back(ev("2026-10-19 00:12:00", "join", "Carol", "Midnight Rooftop", true, false));
    v
}

fn out(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(name)
}

#[test]
fn toast_renders() {
    let mut t = Toast::new("join", "Alice さんが参加しました", "The Black Cat");
    t.fr = true;
    let c = toast::render(&t);
    assert_eq!(c.to_rgba().len(), (toast::W * toast::H * 4) as usize);
    c.save_png(&out("vwn-test-toast.png")).unwrap();
}

#[test]
fn panel_renders_and_switches_notify() {
    let mut p = HistoryPanel::new();
    p.set_status(&PanelStatus { log: "full".into(), friends: "nokey".into() });
    let c = p.render(&sample());
    c.save_png(&out("vwn-test-panel.png")).unwrap();
    // 「フレンドのみ」を押す
    assert!(!p.take_notify_change());
    p.pointer_down(190.0 + 300.0, 112.0 + 12.0 + 26.0);
    p.pointer_up(190.0 + 300.0, 112.0 + 12.0 + 26.0);
    assert_eq!(p.notify(), Notify::Friends);
    assert!(p.take_notify_change());
    assert!(!p.take_notify_change());
    HistoryPanel::new().render(&VecDeque::new()).save_png(&out("vwn-test-empty.png")).unwrap();
}

#[test]
fn corner_note_only_for_problems() {
    use vwn::panel::corner_note;
    let st = |l: &str, f: &str| PanelStatus { log: l.into(), friends: f.into() };
    assert_eq!(corner_note(&st("full", "ok")), None);
    assert_eq!(corner_note(&st("full", "off")), None);
    assert_eq!(corner_note(&st("full", "nokey")), Some("VRCHAT_AUTH_KEY 未設定"));
    assert_eq!(corner_note(&st("full", "bad")), Some("VRCHAT_AUTH_KEY が使えません"));
    assert!(corner_note(&st("errors", "ok")).unwrap().contains("Full"));
}

#[test]
fn friends_only_and_off_filter() {
    let t = Toast::new("join", "Bob さんほか 3 人", "が参加");
    let fr = vec!["Alice".to_string()];
    assert!(toast::filter_people(&t, Notify::Off, &fr, "").is_empty());
    assert!(toast::filter_people(&Toast::new("invite", "A", "から招待"), Notify::Off, &[], "").is_empty());
    assert_eq!(toast::filter_people(&t, Notify::All, &fr, "").len(), 1);
    let only = toast::filter_people(&t, Notify::Friends, &fr, "cympfh");
    assert_eq!(only.len(), 2);
    assert!(only[0].me && only[0].title == "cympfh" && only[1].fr && only[1].title == "Alice");
    assert!(toast::filter_people(&t, Notify::Friends, &[], "").is_empty());
}

#[test]
fn history_jsonl_roundtrip_same_format_as_frame() {
    // 0.5 までの Frame の history.jsonl の 1 行をそのまま読めて、書き戻すと同じ形
    let line = r#"{"t":"2026-10-18 21:03:10","k":"join","n":"Alice","w":"The Black Cat","key":"2026-10-18 21:03:10|join|Alice","wid":"wrld_1","f":"1"}"#;
    let e = HistEvent::from_json(line).unwrap();
    assert!(e.fr && !e.me && e.wid == "wrld_1");
    assert_eq!(HistEvent::from_json(&e.to_json()).unwrap(), e);
    let d = std::env::temp_dir().join(format!("vwn-hist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let mut h = vwn::history::History::new(d.clone());
    for e in sample() {
        h.add(e);
    }
    assert!(!h.add(sample()[1].clone())); // 同じ鍵は 1 回だけ
    let mut h2 = vwn::history::History::new(d.clone());
    h2.load();
    assert_eq!(h2.events().len(), sample().len());
    let _ = std::fs::remove_dir_all(&d);
}
