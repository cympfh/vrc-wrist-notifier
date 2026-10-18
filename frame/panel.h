// SPDX-License-Identifier: MIT
// ダッシュボードの「入退室の履歴」パネル。新しい順、日付とワールドごとにまとめて表示する。
#pragma once

#include "draw.h"
#include "history.h"

#include <ctime>
#include <deque>
#include <string>
#include <vector>

class HistoryPanel {
public:
    static constexpr int kW = 1200, kH = 800;
    static constexpr int kThumb = 256;
    enum Filter { All = 0, JoinOnly = 1, LeaveOnly = 2 };

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
        if (rows_.empty()) {
            draw::text(cr, font_, "まだ履歴がありません", "Bold", 34, kW / 2.0, 380, 0, {0.85, 0.87, 0.9}, draw::Center);
            draw::text(cr, font_, "VRChat の設定 → Debug のログ出力を「Full」にして、Windows 側を起動してください", "Regular", 22,
                       kW / 2.0, 440, 0, {0.6, 0.63, 0.68}, draw::Center);
        }
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
    static constexpr int kListTop = 124;
    static constexpr int kTopButton = 3;

    struct Row {
        enum Type { Day, World, Event } type;
        std::string time, text, kind;
    };

    struct Rect { double x, y, w, h; };
    Rect button_rect(int i) const {
        if (i == kTopButton) return {kW - 180 - 3 * 160 - 20, 30, 170, 64};
        return {double(kW - 36 - (3 - i) * 160 + 10), 30, 150, 64};
    }
    int hit(double x, double y) const {
        for (int i = 0; i < 4; i++) {
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

    void build_rows(const std::deque<HistEvent>& events) {
        // ワールドに入ったところで区切った「滞在」を作り、新しい滞在から順に並べる
        struct Visit { std::string world, start; std::vector<const HistEvent*> items; };
        std::vector<Visit> visits;
        for (auto& e : events) {
            if (e.kind == "world") { visits.push_back({e.name, e.t, {}}); continue; }
            if (visits.empty() || (!e.world.empty() && e.world != visits.back().world))
                visits.push_back({e.world.empty() ? "（ワールド不明）" : e.world, e.t, {}});
            visits.back().items.push_back(&e);
        }
        rows_.clear();
        std::string last_day;
        for (auto v = visits.rbegin(); v != visits.rend(); ++v) {
            std::string day = v->items.empty() ? v->start.substr(0, 10) : v->items.back()->t.substr(0, 10);
            if (day != last_day) { rows_.push_back({Row::Day, "", day_label(day), ""}); last_day = day; }
            rows_.push_back({Row::World, hm(v->start), v->world, "world"});
            for (auto it = v->items.rbegin(); it != v->items.rend(); ++it) {
                const HistEvent& e = **it;
                if ((filter_ == JoinOnly && e.kind != "join") || (filter_ == LeaveOnly && e.kind != "leave")) continue;
                std::string d = e.t.substr(0, 10);
                if (d != last_day) { rows_.push_back({Row::Day, "", day_label(d), ""}); last_day = d; }
                rows_.push_back({Row::Event, hm(e.t), e.name, e.kind});
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
            draw::text(cr, font_, "🌐 " + r.text, "Bold", 26, 52, y + h / 2 + 1, kW - 300, {0.85, 0.92, 1.0});
            draw::text(cr, font_, r.time + " に入室", "Regular", 22, kW - 76, y + h / 2 + 1, 0, {0.6, 0.72, 0.85}, draw::Right);
        } else {
            draw::text(cr, font_, r.time, "Regular", 26, 60, y + h / 2, 0, {0.62, 0.65, 0.70});
            bool join = r.kind == "join";
            draw::Rgb c = join ? draw::Rgb{0.30, 0.80, 0.45} : draw::Rgb{0.55, 0.56, 0.62};
            cairo_set_source_rgb(cr, c.r, c.g, c.b);
            draw::rounded(cr, 172, y + 12, 92, h - 24, 18);
            cairo_fill(cr);
            draw::text(cr, font_, join ? "参加" : "退出", "Bold", 22, 218, y + h / 2, 0, {0.07, 0.08, 0.10}, draw::Center);
            draw::text(cr, font_, r.text, "Regular", 28, 292, y + h / 2, kW - 366, {0.95, 0.96, 0.98});
        }
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
    Filter filter_ = All;
    double scroll_ = 0, content_h_ = 0;
    int hover_ = -1, pressed_ = -1;
    bool dragging_ = false, drag_moved_ = false;
    double drag_y_ = 0, drag_scroll_ = 0;
};
