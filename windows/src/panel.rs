// SPDX-License-Identifier: MIT
//! ダッシュボードの「入退室の履歴」パネル（1200×800）。新しい順、日付とワールドごとにまとめて表示する。
//! 上の帯に「すべて・参加・退出」の絞り込みと「通知設定」（全員 / フレンドのみ / オフ）。問題があるときだけ右下に注意を出す。
use crate::draw::{self, Align, Canvas, Rgb};
use crate::history::HistEvent;
use crate::toast::Notify;
use std::collections::VecDeque;

pub const W: u32 = 1200;
pub const H: u32 = 800;

const BAR_TOP: f32 = 112.0;
const BAR_H: f32 = 116.0; // 「通知設定」の帯（上の段に切り替え、下の段に補足）
const LIST_TOP: f32 = BAR_TOP + BAR_H + 4.0;
const TOP_BUTTON: i32 = 3;
const SEG0: i32 = 4; // 全員 / フレンドのみ / オフ（4, 5, 6）
const SEG_X: f32 = 190.0;
const SEG_W: f32 = 200.0;
const SEG_H: f32 = 52.0;

/// アプリの様子（問題の注意と、履歴が無いときの案内に使う）
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelStatus {
    /// VRChat のログ出力: full / errors / unknown（まだ出ていない）/ nolog（VRChat のログが無い）
    pub log: String,
    /// フレンド一覧: ok / nokey / bad / none（確認中）/ off
    pub friends: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    All = 0,
    JoinOnly = 1,
    LeaveOnly = 2,
}

#[derive(Clone, Debug)]
enum RowType {
    Day,
    World,
    Event,
}

#[derive(Clone, Debug)]
struct Row {
    ty: RowType,
    time: String,
    text: String,
    kind: String,
    sub: String,
    fr: bool,
    me: bool,
}

pub struct HistoryPanel {
    rows: Vec<Row>,
    status: PanelStatus,
    filter: Filter,
    notify: Notify,
    notify_changed: bool,
    scroll: f32,
    content_h: f32,
    hover: i32,
    pressed: i32,
    dragging: bool,
    drag_y: f32,
    drag_scroll: f32,
}

impl Default for HistoryPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// 問題があるときだけ、右下の隅に小さく出す（問題が無ければ何も出さない）
pub fn corner_note(st: &PanelStatus) -> Option<&'static str> {
    match st.friends.as_str() {
        "nokey" => return Some("VRCHAT_AUTH_KEY 未設定"),
        "bad" => return Some("VRCHAT_AUTH_KEY が使えません"),
        _ => {}
    }
    if st.log == "errors" {
        return Some("VRChat のデバッグログが Full ではありません");
    }
    None
}

fn day_label(t: &str) -> String {
    match chrono::NaiveDate::parse_from_str(t.get(..10).unwrap_or(""), "%Y-%m-%d") {
        Ok(d) => {
            use chrono::Datelike;
            const WD: [&str; 7] = ["月", "火", "水", "木", "金", "土", "日"];
            format!("{}年{}月{}日（{}）", d.year(), d.month(), d.day(), WD[d.weekday().num_days_from_monday() as usize])
        }
        Err(_) => t.to_string(),
    }
}
fn hm(t: &str) -> String {
    t.get(11..16).unwrap_or(t).to_string()
}
fn day_short(t: &str) -> String {
    match (t.get(5..7).and_then(|m| m.parse::<u32>().ok()), t.get(8..10).and_then(|d| d.parse::<u32>().ok())) {
        (Some(m), Some(d)) => format!("{m}/{d} "),
        _ => String::new(),
    }
}
fn minutes(t: &str) -> i64 {
    chrono::NaiveDateTime::parse_from_str(t.get(..16).unwrap_or(""), "%Y-%m-%d %H:%M").map_or(0, |d| d.and_utc().timestamp() / 60)
}

struct R {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl HistoryPanel {
    pub fn new() -> HistoryPanel {
        HistoryPanel {
            rows: vec![],
            status: PanelStatus::default(),
            filter: Filter::All,
            notify: Notify::All,
            notify_changed: false,
            scroll: 0.0,
            content_h: 0.0,
            hover: -1,
            pressed: -1,
            dragging: false,
            drag_y: 0.0,
            drag_scroll: 0.0,
        }
    }

    // ------------------------------------------------------------ 入力（座標はパネルの px、左上が原点）。描き直しが要るなら true
    pub fn pointer_move(&mut self, x: f32, y: f32) -> bool {
        let mut changed = false;
        let h = self.hit(x, y);
        if h != self.hover {
            self.hover = h;
            changed = true;
        }
        if self.dragging {
            let dy = y - self.drag_y;
            changed |= self.set_scroll(self.drag_scroll - dy);
        }
        changed
    }
    pub fn pointer_down(&mut self, x: f32, y: f32) -> bool {
        let h = self.hit(x, y);
        if h >= 0 {
            self.pressed = h;
            return true;
        }
        if y > LIST_TOP {
            self.dragging = true;
            self.drag_y = y;
            self.drag_scroll = self.scroll;
        }
        false
    }
    pub fn pointer_up(&mut self, x: f32, y: f32) -> bool {
        self.dragging = false;
        let h = self.hit(x, y);
        let mut changed = false;
        if self.pressed >= 0 && h == self.pressed {
            if h <= 2 {
                self.filter = [Filter::All, Filter::JoinOnly, Filter::LeaveOnly][h as usize];
                self.scroll = 0.0;
            } else if (SEG0..SEG0 + 3).contains(&h) {
                let n = [Notify::All, Notify::Friends, Notify::Off][(h - SEG0) as usize];
                if n != self.notify {
                    self.notify = n;
                    self.notify_changed = true;
                }
            } else if h == TOP_BUTTON {
                self.scroll = 0.0;
            }
            changed = true;
        }
        self.pressed = -1;
        changed
    }
    pub fn pointer_leave(&mut self) -> bool {
        let changed = self.hover != -1 || self.pressed != -1;
        self.hover = -1;
        self.pressed = -1;
        self.dragging = false;
        changed
    }
    /// 上へ倒すと上へ
    pub fn scroll(&mut self, ydelta: f32) -> bool {
        self.set_scroll(self.scroll - ydelta * 120.0)
    }
    pub fn set_filter(&mut self, f: Filter) {
        self.filter = f;
    }
    pub fn set_notify(&mut self, n: Notify) {
        self.notify = n;
    }
    pub fn notify(&self) -> Notify {
        self.notify
    }
    /// パネルで選び直されたら true（1 回だけ）。呼んだ側が設定ファイルに書く
    pub fn take_notify_change(&mut self) -> bool {
        std::mem::take(&mut self.notify_changed)
    }
    pub fn set_status(&mut self, st: &PanelStatus) -> bool {
        if *st == self.status {
            return false;
        }
        self.status = st.clone();
        true
    }

    fn button_rect(&self, i: i32) -> R {
        let w = W as f32;
        if i == TOP_BUTTON {
            return R { x: w - 180.0 - 3.0 * 160.0 - 20.0, y: 30.0, w: 170.0, h: 64.0 };
        }
        if i >= SEG0 {
            return R { x: SEG_X + (i - SEG0) as f32 * SEG_W, y: BAR_TOP + 12.0, w: SEG_W, h: SEG_H };
        }
        R { x: w - 36.0 - (3 - i) as f32 * 160.0 + 10.0, y: 30.0, w: 150.0, h: 64.0 }
    }
    fn hit(&self, x: f32, y: f32) -> i32 {
        for i in 0..SEG0 + 3 {
            if i == TOP_BUTTON && self.scroll <= 1.0 {
                continue;
            }
            let r = self.button_rect(i);
            if x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h {
                return i;
            }
        }
        -1
    }

    // ------------------------------------------------------------ 描画
    pub fn render(&mut self, events: &VecDeque<HistEvent>) -> Canvas {
        self.build_rows(events);
        let (w, h) = (W as f32, H as f32);
        let mut c = Canvas::new(W, H);
        c.clear(Rgb(0.07, 0.08, 0.10));
        // 一覧
        self.content_h = self.rows.iter().map(row_h).sum();
        self.set_scroll(self.scroll);
        let mut y = LIST_TOP + 8.0 - self.scroll;
        let rows = std::mem::take(&mut self.rows);
        for r in &rows {
            let rh = row_h(r);
            if y + rh >= LIST_TOP && y <= h {
                draw_row(&mut c, r, y, rh);
            }
            y += rh;
        }
        let empty = rows.is_empty();
        self.rows = rows;
        if empty {
            self.draw_empty(&mut c);
        }
        // スクロールバー
        let view = h - LIST_TOP;
        if self.content_h > view {
            let bar_h = (view * view / self.content_h).max(60.0);
            let bar_y = LIST_TOP + (view - bar_h) * (self.scroll / (self.content_h - view + 16.0));
            c.rounded(w - 18.0, bar_y + 4.0, 10.0, bar_h - 8.0, 5.0, draw::WHITE, 0.25);
        }
        // 見出しの帯（一覧の上にかぶせる）
        c.rect(0.0, 0.0, w, LIST_TOP, Rgb(0.11, 0.12, 0.15), 1.0);
        c.rect(0.0, LIST_TOP - 2.0, w, 2.0, draw::WHITE, 0.08);
        c.text("入退室の履歴", true, 36.0, 36.0, 46.0, 0.0, draw::WHITE, Align::Left);
        self.draw_notify_bar(&mut c);
        let joins = events.iter().filter(|e| e.kind == "join").count();
        let leaves = events.iter().filter(|e| e.kind == "leave").count();
        c.text(&format!("参加 {joins} 件 ・ 退出 {leaves} 件"), false, 22.0, 38.0, 92.0, 0.0, Rgb(0.6, 0.63, 0.68), Align::Left);
        for (i, label) in ["すべて", "参加", "退出"].iter().enumerate() {
            self.button(&mut c, i as i32, label, self.filter as usize == i);
        }
        if self.scroll > 1.0 {
            self.button(&mut c, TOP_BUTTON, "↑ 最新", false);
        }
        self.draw_corner_note(&mut c);
        c
    }

    fn button(&self, c: &mut Canvas, i: i32, label: &str, selected: bool) {
        let r = self.button_rect(i);
        let (hov, prs) = (self.hover == i, self.pressed == i);
        if selected {
            c.rounded(r.x, r.y, r.w, r.h, 32.0, Rgb(0.30, 0.55, 0.95), 1.0);
        } else {
            c.rounded(r.x, r.y, r.w, r.h, 32.0, draw::WHITE, if prs { 0.30 } else if hov { 0.18 } else { 0.08 });
        }
        c.text(label, true, 26.0, r.x + r.w / 2.0, r.y + r.h / 2.0, 0.0, draw::WHITE, Align::Center);
    }

    /// 「通知設定」: 角丸の溝の中に 3 つ並べ、選んだものだけ色の付いたつまみにする
    fn draw_notify_bar(&self, c: &mut Canvas) {
        let cy = BAR_TOP + 12.0 + SEG_H / 2.0;
        c.text("通知設定", true, 26.0, 36.0, cy, 0.0, Rgb(0.85, 0.87, 0.9), Align::Left);
        let a = self.button_rect(SEG0);
        c.rounded(a.x - 5.0, a.y - 5.0, 3.0 * SEG_W + 10.0, SEG_H + 10.0, (SEG_H + 10.0) / 2.0, draw::WHITE, 0.07);
        let labels = ["全員", "フレンドのみ", "オフ"];
        let on = [Rgb(0.30, 0.55, 0.95), Rgb(0.85, 0.66, 0.12), Rgb(0.45, 0.47, 0.53)];
        let cur = match self.notify {
            Notify::All => 0,
            Notify::Friends => 1,
            Notify::Off => 2,
        };
        for i in 0..3 {
            let r = self.button_rect(SEG0 + i as i32);
            let (sel, hov, prs) = (cur == i, self.hover == SEG0 + i as i32, self.pressed == SEG0 + i as i32);
            if sel {
                c.rounded(r.x + 2.0, r.y, r.w - 4.0, r.h, r.h / 2.0, on[i], 1.0);
            } else if prs || hov {
                c.rounded(r.x + 2.0, r.y, r.w - 4.0, r.h, r.h / 2.0, draw::WHITE, if prs { 0.22 } else { 0.12 });
            }
            let tc = if sel { draw::WHITE } else { Rgb(0.70, 0.72, 0.77) };
            c.text(labels[i], true, 22.0, r.x + r.w / 2.0, r.y + r.h / 2.0, 0.0, tc, Align::Center);
        }
        // 下の段: 補足（キーが未設定・使えないときは右下の注意だけで知らせる）
        let (hx, ny) = (36.0, BAR_TOP + 12.0 + SEG_H + 28.0);
        let mut note = "履歴には全員残ります";
        let mut nc = Rgb(0.6, 0.63, 0.68);
        match self.notify {
            Notify::Friends => {
                match self.status.friends.as_str() {
                    "off" => note = "⚠ フレンドの判定がオフです（履歴には全員残ります）",
                    "none" => note = "フレンド一覧を取得中です（履歴には全員残ります）",
                    _ => {}
                }
                if note.starts_with('⚠') {
                    nc = Rgb(1.0, 0.72, 0.25);
                }
            }
            Notify::Off => note = "通知はすべて止まります（履歴には全員残ります）",
            Notify::All => {}
        }
        c.text(note, false, 20.0, hx, ny, W as f32 - hx - 30.0, nc, Align::Left);
    }

    fn draw_corner_note(&self, c: &mut Canvas) {
        let Some(msg) = corner_note(&self.status) else { return };
        // 履歴が無いときにログ出力のことは、真ん中の案内が同じことを言う
        if self.rows.is_empty() && self.status.log == "errors" && !matches!(self.status.friends.as_str(), "nokey" | "bad") {
            return;
        }
        let text = format!("⚠ {msg}");
        let tw = draw::measure(&text, false, 18.0);
        let (bw, bh) = (tw + 32.0, 38.0);
        let (bx, by) = (W as f32 - bw - 30.0, H as f32 - bh - 16.0);
        c.rounded(bx, by, bw, bh, bh / 2.0, Rgb(0.16, 0.12, 0.05), 0.92);
        c.text(&text, false, 18.0, bx + 16.0, by + bh / 2.0, 0.0, Rgb(1.0, 0.72, 0.25), Align::Left);
    }

    fn build_rows(&mut self, events: &VecDeque<HistEvent>) {
        // ワールドに入ったところで区切った「滞在」を作り、新しい滞在から順に並べる
        struct Visit<'a> {
            world: String,
            start: String,
            items: Vec<&'a HistEvent>,
        }
        const UNKNOWN: &str = "（ワールド不明）";
        let mut visits: Vec<Visit> = vec![];
        for e in events {
            if e.kind == "world" {
                visits.push(Visit { world: e.name.clone(), start: e.t.clone(), items: vec![] });
                continue;
            }
            let mut new_visit = visits.last().map_or(true, |v| !e.world.is_empty() && e.world != v.world);
            if let Some(v) = visits.last() {
                if !new_visit && e.world.is_empty() && v.world != UNKNOWN {
                    // ワールドのわからない記録（古い版）は、前の滞在から 30 分以上あいていたら別にする
                    let last = v.items.last().map_or(v.start.as_str(), |x| x.t.as_str());
                    new_visit = minutes(&e.t) - minutes(last) > 30;
                }
            }
            if new_visit {
                visits.push(Visit { world: if e.world.is_empty() { UNKNOWN.into() } else { e.world.clone() }, start: e.t.clone(), items: vec![] });
            }
            visits.last_mut().unwrap().items.push(e);
        }
        let row = |ty, time: String, text: String, kind: &str, sub: String| Row { ty, time, text, kind: kind.into(), sub, fr: false, me: false };
        let mut rows = vec![];
        let mut last_day = String::new();
        for v in visits.iter().rev() {
            let day = v.items.last().map_or(v.start.as_str(), |x| x.t.as_str()).get(..10).unwrap_or("").to_string();
            if day != last_day {
                rows.push(row(RowType::Day, String::new(), day_label(&day), "", String::new()));
                last_day = day;
            }
            // 見出しの右: いた時間（最初〜最後の出来事）と、来た人の数（自分を含む）
            let mut sub = hm(&v.start) + if v.world == UNKNOWN { "" } else { " に入室" };
            if let Some(end) = v.items.last() {
                if end.t.get(..16) != v.start.get(..16) {
                    let e = &end.t;
                    let same = e.get(..10) == v.start.get(..10);
                    sub = format!("{} 〜 {}", hm(&v.start), if same { hm(e) } else { day_short(e) + &hm(e) });
                }
            }
            let mut names: Vec<&str> = vec![];
            for e in &v.items {
                if e.kind == "join" && !names.contains(&e.name.as_str()) {
                    names.push(&e.name);
                }
            }
            if !names.is_empty() {
                sub += &format!(" ・ {} 人", names.len());
            }
            rows.push(row(RowType::World, hm(&v.start), v.world.clone(), "world", sub));
            for e in v.items.iter().rev() {
                if (self.filter == Filter::JoinOnly && e.kind != "join") || (self.filter == Filter::LeaveOnly && e.kind != "leave") {
                    continue;
                }
                let d = e.t.get(..10).unwrap_or("").to_string();
                if d != last_day {
                    rows.push(row(RowType::Day, String::new(), day_label(&d), "", String::new()));
                    last_day = d;
                }
                rows.push(Row { ty: RowType::Event, time: hm(&e.t), text: e.name.clone(), kind: e.kind.clone(), sub: String::new(), fr: e.fr, me: e.me });
            }
        }
        self.rows = rows;
    }

    /// 履歴がまだ無いとき: VRChat のデバッグログが Full になっているかを 1 行で
    fn draw_empty(&self, c: &mut Canvas) {
        let w = W as f32;
        let dy = LIST_TOP - 124.0;
        c.text("まだ履歴がありません", true, 34.0, w / 2.0, 250.0 + dy, 0.0, Rgb(0.85, 0.87, 0.9), Align::Center);
        let dl = "VRChat のデバッグログ（Full）";
        let (state, hint, level) = match self.status.log.as_str() {
            "full" => ("設定済み", "", 0),
            "errors" => ("設定されていません", "VRChat の設定 → Debug のログ出力を「Full」にしてください", 1),
            "nolog" => ("まだ確認できません", "VRChat を起動すると確認します", 2),
            _ => ("まだ確認できません", "VRChat のログがまだ出ていません。少し待つと確認します", 2),
        };
        let col = match level {
            0 => Rgb(0.35, 0.85, 0.50),
            1 => Rgb(1.0, 0.72, 0.25),
            _ => Rgb(0.62, 0.65, 0.70),
        };
        let y = 340.0 + dy;
        c.rounded(60.0, y - 44.0, w - 120.0, if hint.is_empty() { 88.0 } else { 128.0 }, 20.0, draw::WHITE, 0.05);
        let mark = ["✔ ", "! ", "? "][level];
        c.text(&format!("{mark}{dl}：{state}"), true, 28.0, 96.0, y, w - 180.0, col, Align::Left);
        if !hint.is_empty() {
            c.text(hint, false, 22.0, 128.0, y + 46.0, w - 210.0, Rgb(0.75, 0.78, 0.82), Align::Left);
        }
        if level == 0 {
            c.text("準備はできています。だれかが参加・退出すると、ここに記録されます", false, 22.0, w / 2.0, y + 130.0, 0.0, Rgb(0.6, 0.63, 0.68), Align::Center);
        }
    }

    fn set_scroll(&mut self, s: f32) -> bool {
        let max = (self.content_h - (H as f32 - LIST_TOP) + 16.0).max(0.0);
        let s = s.clamp(0.0, max);
        let changed = (s - self.scroll).abs() > 0.5;
        self.scroll = s;
        changed
    }
}

fn row_h(r: &Row) -> f32 {
    match r.ty {
        RowType::Day => 70.0,
        RowType::World => 72.0,
        RowType::Event => 60.0,
    }
}

fn draw_row(c: &mut Canvas, r: &Row, y: f32, h: f32) {
    let w = W as f32;
    match r.ty {
        RowType::Day => {
            c.text(&r.text, true, 28.0, 36.0, y + h / 2.0 + 6.0, 0.0, draw::WHITE, Align::Left);
            c.rect(36.0, y + h - 6.0, w - 90.0, 2.0, draw::WHITE, 0.12);
        }
        RowType::World => {
            c.rounded(30.0, y + 8.0, w - 84.0, h - 14.0, 16.0, Rgb(0.13, 0.20, 0.30), 1.0);
            c.text(&format!("🌐 {}", r.text), true, 26.0, 52.0, y + h / 2.0 + 1.0, w - 420.0, Rgb(0.85, 0.92, 1.0), Align::Left);
            c.text(&r.sub, false, 22.0, w - 76.0, y + h / 2.0 + 1.0, 0.0, Rgb(0.6, 0.72, 0.85), Align::Right);
        }
        RowType::Event => {
            c.text(&r.time, false, 26.0, 60.0, y + h / 2.0, 0.0, Rgb(0.62, 0.65, 0.70), Align::Left);
            let join = r.kind == "join";
            let col = if join { Rgb(0.30, 0.80, 0.45) } else { Rgb(0.55, 0.56, 0.62) };
            c.rounded(172.0, y + 12.0, 92.0, h - 24.0, 18.0, col, 1.0);
            c.text(if join { "参加" } else { "退出" }, true, 22.0, 218.0, y + h / 2.0, 0.0, Rgb(0.07, 0.08, 0.10), Align::Center);
            let (bold, col) = if r.me { (true, draw::SELF) } else if r.fr { (true, draw::FRIEND) } else { (false, Rgb(0.95, 0.96, 0.98)) };
            c.text(&r.text, bold, 28.0, 292.0, y + h / 2.0, w - 366.0, col, Align::Left);
        }
    }
}
