// SPDX-License-Identifier: MIT
//! 通知のカード（640×160）。色分け、フレンドは黄色・自分は水色、「通知設定」での選び方。
use crate::draw::{self, Align, Canvas, Rgb};

pub const W: u32 = 640;
pub const H: u32 = 160;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toast {
    pub kind: String,
    pub title: String,
    pub text: String,
    /// 先頭の名前がフレンド（まとめカードなら「〜 さんほか N 人」の名前の部分だけ黄色）
    pub fr: bool,
    /// 先頭の名前が自分（水色）
    pub me: bool,
}

impl Toast {
    pub fn new(kind: &str, title: &str, text: &str) -> Toast {
        Toast { kind: kind.into(), title: title.into(), text: text.into(), ..Default::default() }
    }
}

/// 外から来た文字列を表示・保存してよい形にする: 制御文字（改行・タブを含む）を空白にし、max 文字までにする
pub fn clean(s: &str, max: usize) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().to_string()
}

/// 通知設定: 全員 / フレンドのみ（参加・退出をフレンドと自分だけに）/ オフ（どの通知も出さない）。履歴には関係なく全員残る
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notify {
    All,
    Friends,
    Off,
}

impl Notify {
    pub fn name(self) -> &'static str {
        match self {
            Notify::All => "all",
            Notify::Friends => "friends",
            Notify::Off => "off",
        }
    }
    pub fn from(v: &str) -> Notify {
        match v {
            "friends" => Notify::Friends,
            "off" => Notify::Off,
            _ => Notify::All,
        }
    }
}

/// まとめカードのタイトル「名前 さんほか N 人」を、名前と残りに分ける（まとめでなければ残りは空）
pub fn split_summary(title: &str) -> (&str, &str) {
    match title.rfind(" さんほか ") {
        Some(i) if i > 0 => (&title[..i], &title[i..]),
        _ => (title, ""),
    }
}

/// 「通知設定」でカードを選ぶ。オフならどの種類も出さない。フレンドのみは参加・退出だけに効き、ほかの種類はそのまま。
/// friends / self_name: そのカードに入っているフレンドの名前と自分の名前。
/// フレンドのみのときは、自分とフレンドだけで数えなおす（自分が先頭。2 人までは 1 人ずつ、3 人以上は「名前 さんほか N 人」）
pub fn filter_people(t: &Toast, mode: Notify, friends: &[String], self_name: &str) -> Vec<Toast> {
    // トレイの「テスト通知」は自分で押したものなので、通知設定（フレンドのみ・オフ）に関わらず出す
    if is_test(&t.kind) {
        return vec![t.clone()];
    }
    if mode == Notify::Off {
        return vec![];
    }
    if (t.kind != "join" && t.kind != "leave") || mode != Notify::Friends {
        return vec![t.clone()];
    }
    let verb = if t.kind == "join" { "参加" } else { "退出" };
    let mut people: Vec<&str> = Vec::new();
    if !self_name.is_empty() {
        people.push(self_name);
    }
    people.extend(friends.iter().map(|s| s.as_str()));
    let is_me = |i: usize| i == 0 && !self_name.is_empty();
    if people.len() >= 3 {
        return vec![Toast {
            kind: t.kind.clone(),
            title: format!("{} さんほか {} 人", people[0], people.len() - 1),
            text: format!("が{verb}"),
            fr: !is_me(0),
            me: is_me(0),
        }];
    }
    people
        .iter()
        .enumerate()
        .map(|(i, p)| Toast { kind: t.kind.clone(), title: p.to_string(), text: format!("が{verb}しました"), fr: !is_me(i), me: is_me(i) })
        .collect()
}

// 種類は「基本.詳細」でもよい（例: alert.temp、care.break）。色は基本で、アイコンは詳細で決める。
// alert / urgent = 赤、warn = 琥珀、care / info = 青緑
fn kind_base(kind: &str) -> &str {
    kind.split('.').next().unwrap_or(kind)
}
fn kind_sub(kind: &str) -> &str {
    kind.split_once('.').map_or("", |x| x.1)
}
/// トレイの「テスト通知」の種類（色とアイコンは info と同じ）
pub const TEST_KIND: &str = "info.test";
pub fn is_test(kind: &str) -> bool {
    kind == TEST_KIND
}

/// 「テスト通知」を押したとき、トレイに出す一言（SteamVR につながっていれば VR に出るので無し）
pub fn test_feedback(vr_connected: bool) -> Option<&'static str> {
    (!vr_connected).then_some("SteamVR が動いていないので、VR には出せません。SteamVR を起動してから試してください")
}

pub fn is_alert(kind: &str) -> bool {
    matches!(kind_base(kind), "alert" | "urgent")
}

pub fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "join" => return "➕",
        "leave" => return "➖",
        "world" => return "🌐",
        "invite" | "requestInvite" => return "✉",
        "friendRequest" => return "★",
        _ => {}
    }
    match kind_sub(kind) {
        "temp" => return "🌡",
        "battery" => return "🔋",
        "charge" => return "⚡",
        "break" => return "☕",
        "water" => return "💧",
        _ => {}
    }
    if is_alert(kind) {
        return "!";
    }
    match kind_base(kind) {
        "warn" => "⚠",
        "care" | "info" => "♥",
        _ => "●",
    }
}

pub fn kind_color(kind: &str) -> Rgb {
    match kind {
        "join" => Rgb(0.30, 0.80, 0.45),                                   // 緑
        "leave" => Rgb(0.60, 0.60, 0.65),                                  // 灰
        "world" => Rgb(0.30, 0.70, 0.95),
        "invite" | "requestInvite" | "friendRequest" => Rgb(0.98, 0.70, 0.25),
        k if is_alert(k) => Rgb(0.93, 0.27, 0.27),                         // 赤
        k if kind_base(k) == "warn" => Rgb(1.00, 0.58, 0.10),              // 琥珀
        k if matches!(kind_base(k), "care" | "info") => Rgb(0.18, 0.76, 0.70), // 青緑
        _ => Rgb(0.35, 0.55, 0.95),                                        // その他: 青
    }
}

pub fn render(t: &Toast) -> Canvas {
    let mut c = Canvas::new(W, H);
    let (w, h) = (W as f32, H as f32);
    let alert = is_alert(&t.kind);
    if alert {
        c.rounded(4.0, 4.0, w - 8.0, h - 8.0, 28.0, Rgb(0.22, 0.06, 0.07), 0.94); // 急ぎは背景も赤み
    } else {
        c.rounded(4.0, 4.0, w - 8.0, h - 8.0, 28.0, Rgb(0.08, 0.09, 0.11), 0.90);
    }
    let kc = kind_color(&t.kind);
    c.rounded_stroke(4.0, 4.0, w - 8.0, h - 8.0, 28.0, if alert { 8.0 } else { 4.0 }, kc, 0.9);
    c.circle(76.0, h / 2.0, 46.0, kc, 0.95);
    c.text(kind_icon(&t.kind), true, 34.0, 76.0, h / 2.0, 0.0, Rgb(0.08, 0.09, 0.11), Align::Center);
    let maxw = w - 170.0;
    if t.fr || t.me {
        let nc = if t.me { draw::SELF } else { draw::FRIEND };
        let (name, rest) = split_summary(&t.title);
        if !rest.is_empty() {
            // 「さんほか N 人」は白のまま、名前だけ色を付ける。名前が長すぎるときは名前の側を省略する
            let rw = draw::measure(rest, true, 26.0);
            let nw = draw::measure(name, true, 26.0);
            let namew = (nw + 1.0).min((maxw - rw).max(60.0));
            let drawn = c.text(name, true, 26.0, 142.0, 50.0, namew, nc, Align::Left);
            let used = drawn.min(namew);
            c.text(rest, true, 26.0, 142.0 + used, 50.0, maxw - used, draw::WHITE, Align::Left);
        } else {
            c.text(name, true, 26.0, 142.0, 50.0, maxw, nc, Align::Left);
        }
    } else {
        c.text(&t.title, true, 26.0, 142.0, 50.0, maxw, draw::WHITE, Align::Left);
    }
    c.text(&t.text, false, 22.0, 142.0, 104.0, maxw, Rgb(0.82, 0.84, 0.88), Align::Left);
    c
}
