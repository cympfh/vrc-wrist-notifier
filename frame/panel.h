// SPDX-License-Identifier: MIT
// ダッシュボードの「入退室の履歴」パネル。新しい順、日付とワールドごとにまとめて表示する。
#pragma once

#include "draw.h"
#include "history.h"

#include <algorithm>
#include <ctime>
#include <deque>
#include <string>
#include <vector>

// Windows 側の見張りの様子（履歴がまだ無いときに、準備が済んでいるかを出すのに使う）
struct WatchStatus {
    enum Watcher { Off, Old, On } watcher = Off;   // Old: 0.3.0 より前の Windows 側（通知は届くが、様子は知らせてこない）
    std::string version;
    std::string log;   // VRChat のログ出力: full / errors / unknown（まだ出ていない）/ nolog（VRChat のログが無い）
    std::string friends;   // フレンド一覧（0.4.1 から）: ok / nokey / bad / none（まだ無い）/ off。空なら古い Windows 側
    bool operator==(const WatchStatus& o) const {
        return watcher == o.watcher && version == o.version && log == o.log && friends == o.friends;
    }
};

class HistoryPanel {
public:
    static constexpr int kW = 1200, kH = 800;
    static constexpr int kThumb = 256;
    enum Filter { All = 0, JoinOnly = 1, LeaveOnly = 2 };
    // 参加・退出の通知カードを出す相手（履歴には関係なく全員残る）
    enum Notify { NotifyAll = 0, NotifyFriends = 1, NotifyOff = 2 };
    static const char* notify_name(Notify n) { return n == NotifyFriends ? "friends" : n == NotifyOff ? "off" : "all"; }
    static Notify notify_from(const std::string& v) { return v == "friends" ? NotifyFriends : v == "off" ? NotifyOff : NotifyAll; }

    explicit HistoryPanel(std::string font) : font_(std::move(font)) {}

    // ---------------------------------------------------------------- 入力（座標はパネルの px、左上が原点）
    // 何か変わって描き直しが要るなら true
    bool pointer_move(double x, double y) {
        bool changed = false;
        int h = hit(x, y);
        if (h != hover_) { hover_ = h; changed = true; }
        if (dragging_) {
            double dy = y - drag_y_;
            if (std::abs(dy) > 4) drag_moved_ = true;
            changed |= set_scroll(drag_scroll_ - dy);
        }
        return changed;
    }
    bool pointer_down(double x, double y) {
        int h = hit(x, y);
        if (h >= 0) {
            pressed_ = h;
            return true;
        }
        if (y > kListTop) {
            dragging_ = true;
            drag_moved_ = false;
            drag_y_ = y;
            drag_scroll_ = scroll_;
        }
        return false;
    }
    bool pointer_up(double x, double y) {
        dragging_ = false;
        int h = hit(x, y);
        bool changed = false;
        if (pressed_ >= 0 && h == pressed_) {
            if (h <= LeaveOnly) { filter_ = Filter(h); scroll_ = 0; }
            else if (h >= kSeg0 && h < kSeg0 + 3 && Notify(h - kSeg0) != notify_) { notify_ = Notify(h - kSeg0); notify_changed_ = true; }
            else if (h == kTopButton) scroll_ = 0;
            changed = true;
        }
        pressed_ = -1;
        return changed;
    }
    bool pointer_leave() {
        bool changed = hover_ != -1 || pressed_ != -1;
        hover_ = pressed_ = -1;
        dragging_ = false;
        return changed;
    }
    bool scroll(double ydelta) { return set_scroll(scroll_ - ydelta * 120); }   // 上へ倒すと上へ
    void set_filter(Filter f) { filter_ = f; }
    void set_notify(Notify n) { notify_ = n; }
    Notify notify() const { return notify_; }
    // パネルで選び直されたら true（1 回だけ）。呼んだ側が設定ファイルに書く
    bool take_notify_change() { bool c = notify_changed_; notify_changed_ = false; return c; }
    // 変わったら true（描き直しが要る）
    bool set_status(const WatchStatus& st) {
        if (st == status_) return false;
        status_ = st;
        return true;
    }

    // ---------------------------------------------------------------- 描画
    std::vector<uint8_t> render(const std::deque<HistEvent>& events, cairo_surface_t** keep = nullptr) {
        build_rows(events);
        cairo_surface_t* s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, kW, kH);
        cairo_t* cr = cairo_create(s);
        // 背景
        cairo_set_source_rgb(cr, 0.07, 0.08, 0.10);
        cairo_paint(cr);

        // 一覧
        content_h_ = 0;
        for (auto& r : rows_) content_h_ += row_h(r);
        set_scroll(scroll_);
        cairo_save(cr);
        cairo_rectangle(cr, 0, kListTop, kW, kH - kListTop);
        cairo_clip(cr);
        double y = kListTop + 8 - scroll_;
        for (auto& r : rows_) {
            double h = row_h(r);
            if (y + h >= kListTop && y <= kH) draw_row(cr, r, y, h);
            y += h;
        }
        if (rows_.empty()) draw_empty(cr);
        cairo_restore(cr);

        // スクロールバー
        double view = kH - kListTop;
        if (content_h_ > view) {
            double bar_h = std::max(60.0, view * view / content_h_);
            double bar_y = kListTop + (view - bar_h) * (scroll_ / (content_h_ - view + 16));
            cairo_set_source_rgba(cr, 1, 1, 1, 0.25);
            draw::rounded(cr, kW - 18, bar_y + 4, 10, bar_h - 8, 5);
            cairo_fill(cr);
        }

        // 見出しの帯
        cairo_set_source_rgb(cr, 0.11, 0.12, 0.15);
        cairo_rectangle(cr, 0, 0, kW, kListTop);
        cairo_fill(cr);
        cairo_set_source_rgba(cr, 1, 1, 1, 0.08);
        cairo_rectangle(cr, 0, kListTop - 2, kW, 2);
        cairo_fill(cr);
        draw::text(cr, font_, "入退室の履歴", "Bold", 36, 36, 46, 0, {1, 1, 1});
        draw_notify_bar(cr);
        int joins = 0, leaves = 0;
        for (auto& e : events) { joins += e.kind == "join"; leaves += e.kind == "leave"; }
        draw::text(cr, font_, "参加 " + std::to_string(joins) + " 件 ・ 退出 " + std::to_string(leaves) + " 件", "Regular", 22,
                   38, 92, 0, {0.6, 0.63, 0.68});
        static const char* labels[] = {"すべて", "参加", "退出"};
        for (int i = 0; i < 3; i++) button(cr, i, labels[i], filter_ == i);
        if (scroll_ > 1) button(cr, kTopButton, "↑ 最新", false);

        cairo_destroy(cr);
        std::vector<uint8_t> px = draw::to_rgba(s);
        if (keep) *keep = s;
        else cairo_surface_destroy(s);
        return px;
    }

    // ダッシュボードの下に並ぶアイコン
    std::vector<uint8_t> render_thumbnail(cairo_surface_t** keep = nullptr) {
        cairo_surface_t* s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, kThumb, kThumb);
        cairo_t* cr = cairo_create(s);
        cairo_set_source_rgb(cr, 0.10, 0.11, 0.14);
        draw::rounded(cr, 8, 8, kThumb - 16, kThumb - 16, 48);
        cairo_fill(cr);
        draw::Rgb colors[] = {{0.30, 0.80, 0.45}, {0.60, 0.60, 0.65}, {0.30, 0.80, 0.45}};
        for (int i = 0; i < 3; i++) {
            double cy = 72 + i * 56;
            cairo_set_source_rgb(cr, colors[i].r, colors[i].g, colors[i].b);
            cairo_arc(cr, 64, cy, 16, 0, 2 * M_PI);
            cairo_fill(cr);
            cairo_set_source_rgba(cr, 1, 1, 1, 0.85);
            draw::rounded(cr, 96, cy - 9, i == 1 ? 90 : 120, 18, 9);
            cairo_fill(cr);
        }
        cairo_destroy(cr);
        std::vector<uint8_t> px = draw::to_rgba(s);
        if (keep) *keep = s;
        else cairo_surface_destroy(s);
        return px;
    }

private:
    static constexpr int kBarTop = 116, kBarH = 76;   // 「通知する人」の帯
    static constexpr int kListTop = kBarTop + kBarH + 4;
    static constexpr int kTopButton = 3;
    static constexpr int kSeg0 = 4;                     // 全員 / フレンドのみ / オフ（4, 5, 6）
    static constexpr int kSegX = 254, kSegW = 186, kSegH = 52;

    struct Row {
        enum Type { Day, World, Event } type;
        std::string time, text, kind, sub;   // sub: ワールドの見出しの右側
        bool fr = false;                     // フレンドなら名前を黄色に
    };

    struct Rect { double x, y, w, h; };
    Rect button_rect(int i) const {
        if (i == kTopButton) return {kW - 180 - 3 * 160 - 20, 30, 170, 64};
        if (i >= kSeg0) return {double(kSegX + (i - kSeg0) * kSegW), double(kBarTop + (kBarH - kSegH) / 2), double(kSegW), double(kSegH)};
        return {double(kW - 36 - (3 - i) * 160 + 10), 30, 150, 64};
    }
    int hit(double x, double y) const {
        for (int i = 0; i < kSeg0 + 3; i++) {
            if (i == kTopButton && scroll_ <= 1) continue;
            Rect r = button_rect(i);
            if (x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h) return i;
        }
        return -1;
    }
    void button(cairo_t* cr, int i, const char* label, bool selected) {
        Rect r = button_rect(i);
        bool hov = hover_ == i, prs = pressed_ == i;
        if (selected) cairo_set_source_rgb(cr, 0.30, 0.55, 0.95);
        else cairo_set_source_rgba(cr, 1, 1, 1, prs ? 0.30 : hov ? 0.18 : 0.08);
        draw::rounded(cr, r.x, r.y, r.w, r.h, 32);
        cairo_fill(cr);
        draw::text(cr, font_, label, "Bold", 26, r.x + r.w / 2, r.y + r.h / 2, 0, {1, 1, 1}, draw::Center);
    }

    // 「通知」: 角丸の溝の中に 3 つ並べ、選んだものだけ色の付いたつまみにする
    void draw_notify_bar(cairo_t* cr) {
        double cy = kBarTop + kBarH / 2.0;
        draw::text(cr, font_, "参加・退出の通知", "Bold", 24, 36, cy, 0, {0.85, 0.87, 0.9});
        Rect a = button_rect(kSeg0);
        cairo_set_source_rgba(cr, 1, 1, 1, 0.07);
        draw::rounded(cr, a.x - 5, a.y - 5, 3 * kSegW + 10, kSegH + 10, (kSegH + 10) / 2.0);
        cairo_fill(cr);
        static const char* labels[] = {"全員", "フレンドのみ", "オフ"};
        static const draw::Rgb on[] = {{0.30, 0.55, 0.95}, {0.85, 0.66, 0.12}, {0.45, 0.47, 0.53}};
        for (int i = 0; i < 3; i++) {
            Rect r = button_rect(kSeg0 + i);
            bool sel = notify_ == i, hov = hover_ == kSeg0 + i, prs = pressed_ == kSeg0 + i;
            if (sel) cairo_set_source_rgb(cr, on[i].r, on[i].g, on[i].b);
            else cairo_set_source_rgba(cr, 1, 1, 1, prs ? 0.22 : hov ? 0.12 : 0.0);
            draw::rounded(cr, r.x + 2, r.y, r.w - 4, r.h, r.h / 2);
            cairo_fill(cr);
            draw::text(cr, font_, labels[i], "Bold", 22, r.x + r.w / 2, r.y + r.h / 2, 0,
                       sel ? draw::Rgb{1, 1, 1} : draw::Rgb{0.70, 0.72, 0.77}, draw::Center);
        }
        // 右側: 補足（フレンドがわからないときは注意を目立たせる）
        double hx = kSegX + 3 * kSegW + 22;
        std::string note = "履歴には全員残ります";
        draw::Rgb nc{0.6, 0.63, 0.68};
        if (notify_ == NotifyFriends) {
            const std::string& f = status_.friends;
            if (status_.watcher != WatchStatus::On) note = "Windows 側の見張りを待っています";
            else if (f == "nokey") note = "⚠ キーが無くフレンド不明";
            else if (f == "bad") note = "⚠ キーが切れています";
            else if (f == "off") note = "⚠ Windows 側で判定がオフ";
            else if (f == "none") note = "フレンド一覧を取得中…";
            else if (f.empty()) note = "⚠ Windows 側を 0.4.1 以降に";
            if (note.rfind("⚠", 0) == 0) nc = {1.0, 0.72, 0.25};
        } else if (notify_ == NotifyOff) note = "カードは出ません（履歴には残ります）";
        draw::text(cr, font_, note, "Regular", 20, hx, cy, kW - int(hx) - 24, nc);
    }

    static int row_h(const Row& r) { return r.type == Row::Day ? 70 : r.type == Row::World ? 72 : 60; }

    static std::string day_label(const std::string& t) {
        if (t.size() < 10) return t;
        struct tm tm {};
        tm.tm_year = std::stoi(t.substr(0, 4)) - 1900;
        tm.tm_mon = std::stoi(t.substr(5, 2)) - 1;
        tm.tm_mday = std::stoi(t.substr(8, 2));
        tm.tm_hour = 12;
        mktime(&tm);
        static const char* wd[] = {"日", "月", "火", "水", "木", "金", "土"};
        return std::to_string(tm.tm_year + 1900) + "年" + std::to_string(tm.tm_mon + 1) + "月" + std::to_string(tm.tm_mday) +
               "日（" + wd[tm.tm_wday] + "）";
    }
    static std::string hm(const std::string& t) { return t.size() >= 16 ? t.substr(11, 5) : t; }
    static std::string day_short(const std::string& t) {   // "10/18 "
        return t.size() >= 10 ? std::to_string(std::stoi(t.substr(5, 2))) + "/" + std::to_string(std::stoi(t.substr(8, 2))) + " " : "";
    }

    void build_rows(const std::deque<HistEvent>& events) {
        // ワールドに入ったところで区切った「滞在」を作り、新しい滞在から順に並べる
        struct Visit { std::string world, start; std::vector<const HistEvent*> items; };
        std::vector<Visit> visits;
        static const std::string unknown = "（ワールド不明）";
        auto minutes = [](const std::string& t) -> long {   // "YYYY-MM-DD HH:MM" を分に（間隔を比べるだけ）
            struct tm tm {};
            if (t.size() < 16) return 0;
            tm.tm_year = std::stoi(t.substr(0, 4)) - 1900; tm.tm_mon = std::stoi(t.substr(5, 2)) - 1;
            tm.tm_mday = std::stoi(t.substr(8, 2)); tm.tm_hour = std::stoi(t.substr(11, 2)); tm.tm_min = std::stoi(t.substr(14, 2));
            tm.tm_isdst = -1;
            return long(mktime(&tm) / 60);
        };
        for (auto& e : events) {
            if (e.kind == "world") { visits.push_back({e.name, e.t, {}}); continue; }
            bool new_visit = visits.empty() || (!e.world.empty() && e.world != visits.back().world);
            if (!new_visit && e.world.empty() && visits.back().world != unknown) {
                // ワールドのわからない記録（古い版）は、前の滞在から 30 分以上あいていたら別にする
                const std::string& last = visits.back().items.empty() ? visits.back().start : visits.back().items.back()->t;
                new_visit = minutes(e.t) - minutes(last) > 30;
            }
            if (new_visit) visits.push_back({e.world.empty() ? unknown : e.world, e.t, {}});
            visits.back().items.push_back(&e);
        }
        rows_.clear();
        std::string last_day;
        for (auto v = visits.rbegin(); v != visits.rend(); ++v) {
            std::string day = v->items.empty() ? v->start.substr(0, 10) : v->items.back()->t.substr(0, 10);
            if (day != last_day) { rows_.push_back({Row::Day, "", day_label(day), "", ""}); last_day = day; }
            // 見出しの右: いた時間（最初〜最後の出来事）と、来た人の数（自分を含む）
            std::string sub = hm(v->start) + (v->world == unknown ? "" : " に入室");
            if (!v->items.empty() && v->items.back()->t.substr(0, 16) != v->start.substr(0, 16)) {
                std::string end = v->items.back()->t;
                sub = hm(v->start) + " 〜 " + (end.substr(0, 10) == v->start.substr(0, 10) ? hm(end) : day_short(end) + hm(end));
            }
            std::vector<std::string> names;
            for (auto* e : v->items)
                if (e->kind == "join" && std::find(names.begin(), names.end(), e->name) == names.end()) names.push_back(e->name);
            if (!names.empty()) sub += " ・ " + std::to_string(names.size()) + " 人";
            rows_.push_back({Row::World, hm(v->start), v->world, "world", sub});
            for (auto it = v->items.rbegin(); it != v->items.rend(); ++it) {
                const HistEvent& e = **it;
                if ((filter_ == JoinOnly && e.kind != "join") || (filter_ == LeaveOnly && e.kind != "leave")) continue;
                std::string d = e.t.substr(0, 10);
                if (d != last_day) { rows_.push_back({Row::Day, "", day_label(d), "", ""}); last_day = d; }
                rows_.push_back({Row::Event, hm(e.t), e.name, e.kind, "", e.fr});
            }
        }
    }

    void draw_row(cairo_t* cr, const Row& r, double y, double h) {
        if (r.type == Row::Day) {
            draw::text(cr, font_, r.text, "Bold", 28, 36, y + h / 2 + 6, 0, {1, 1, 1});
            cairo_set_source_rgba(cr, 1, 1, 1, 0.12);
            cairo_rectangle(cr, 36, y + h - 6, kW - 90, 2);
            cairo_fill(cr);
        } else if (r.type == Row::World) {
            cairo_set_source_rgb(cr, 0.13, 0.20, 0.30);
            draw::rounded(cr, 30, y + 8, kW - 84, h - 14, 16);
            cairo_fill(cr);
            draw::text(cr, font_, "🌐 " + r.text, "Bold", 26, 52, y + h / 2 + 1, kW - 420, {0.85, 0.92, 1.0});
            draw::text(cr, font_, r.sub, "Regular", 22, kW - 76, y + h / 2 + 1, 0, {0.6, 0.72, 0.85}, draw::Right);
        } else {
            draw::text(cr, font_, r.time, "Regular", 26, 60, y + h / 2, 0, {0.62, 0.65, 0.70});
            bool join = r.kind == "join";
            draw::Rgb c = join ? draw::Rgb{0.30, 0.80, 0.45} : draw::Rgb{0.55, 0.56, 0.62};
            cairo_set_source_rgb(cr, c.r, c.g, c.b);
            draw::rounded(cr, 172, y + 12, 92, h - 24, 18);
            cairo_fill(cr);
            draw::text(cr, font_, join ? "参加" : "退出", "Bold", 22, 218, y + h / 2, 0, {0.07, 0.08, 0.10}, draw::Center);
            if (r.fr) draw::text(cr, font_, r.text, "Bold", 28, 292, y + h / 2, kW - 366, draw::kFriend);
            else draw::text(cr, font_, r.text, "Regular", 28, 292, y + h / 2, kW - 366, {0.95, 0.96, 0.98});
        }
    }

    // 履歴がまだ無いとき: 準備の 2 つが済んでいるかを、それぞれ 1 行で
    void draw_empty(cairo_t* cr) {
        const double dy = kListTop - 124;   // 帯が増えた分だけ下げる
        draw::text(cr, font_, "まだ履歴がありません", "Bold", 34, kW / 2.0, 250 + dy, 0, {0.85, 0.87, 0.9}, draw::Center);
        struct Item { std::string label, state, hint; int level; };   // level: 0 = 済み、1 = 要対応、2 = まだ確認できない
        std::vector<Item> items;
        const WatchStatus& st = status_;
        if (st.watcher == WatchStatus::Off)
            items.push_back({"Windows 側の見張り", "動いていません", "VRChat を動かす PC で vrc-wrist-notifier を起動してください", 1});
        else
            items.push_back({"Windows 側の見張り", "設定済み", "", 0});
        std::string dl = "VRChat のデバッグログ（Full）";
        if (st.watcher == WatchStatus::Off)
            items.push_back({dl, "まだ確認できません", "Windows 側の見張りが動くと確認します", 2});
        else if (st.watcher == WatchStatus::Old)
            items.push_back({dl, "まだ確認できません", "Windows 側を新しい版（0.3.0 以降）にすると確認します", 2});
        else if (st.log == "full")
            items.push_back({dl, "設定済み", "", 0});
        else if (st.log == "errors")
            items.push_back({dl, "設定されていません", "VRChat の設定 → Debug のログ出力を「Full」にしてください", 1});
        else if (st.log == "nolog")
            items.push_back({dl, "まだ確認できません", "VRChat を起動すると確認します", 2});
        else
            items.push_back({dl, "まだ確認できません", "VRChat のログがまだ出ていません。少し待つと確認します", 2});

        double y = 340 + dy;
        for (auto& it : items) {
            draw::Rgb c = it.level == 0 ? draw::Rgb{0.35, 0.85, 0.50} : it.level == 1 ? draw::Rgb{1.0, 0.72, 0.25} : draw::Rgb{0.62, 0.65, 0.70};
            cairo_set_source_rgba(cr, 1, 1, 1, 0.05);
            draw::rounded(cr, 60, y - 44, kW - 120, it.hint.empty() ? 88 : 128, 20);
            cairo_fill(cr);
            draw::text(cr, font_, std::string(it.level == 0 ? "✔ " : it.level == 1 ? "! " : "? ") + it.label + "：" + it.state, "Bold", 28,
                       96, y, kW - 180, c);
            if (!it.hint.empty())
                draw::text(cr, font_, it.hint, "Regular", 22, 128, y + 46, kW - 210, {0.75, 0.78, 0.82});
            y += it.hint.empty() ? 120 : 160;
        }
        bool ready = items[0].level == 0 && items[1].level == 0;
        if (ready)
            draw::text(cr, font_, "準備はできています。だれかが参加・退出すると、ここに記録されます", "Regular", 22, kW / 2.0, y + 10, 0,
                       {0.6, 0.63, 0.68}, draw::Center);
    }

    bool set_scroll(double s) {
        double max = std::max(0.0, content_h_ - (kH - kListTop) + 16);
        s = std::max(0.0, std::min(max, s));
        bool changed = std::abs(s - scroll_) > 0.5;
        scroll_ = s;
        return changed;
    }

    std::string font_;
    std::vector<Row> rows_;
    WatchStatus status_;
    Filter filter_ = All;
    Notify notify_ = NotifyAll;
    bool notify_changed_ = false;
    double scroll_ = 0, content_h_ = 0;
    int hover_ = -1, pressed_ = -1;
    bool dragging_ = false, drag_moved_ = false;
    double drag_y_ = 0, drag_scroll_ = 0;
};
