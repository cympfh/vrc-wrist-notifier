// SPDX-License-Identifier: MIT
// cairo + pango の小さな描画部品
#pragma once

#include <cairo.h>
#include <pango/pangocairo.h>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

namespace draw {

inline void rounded(cairo_t* cr, double x, double y, double w, double h, double r) {
    r = std::min(r, std::min(w, h) / 2);
    cairo_new_sub_path(cr);
    cairo_arc(cr, x + w - r, y + r, r, -M_PI / 2, 0);
    cairo_arc(cr, x + w - r, y + h - r, r, 0, M_PI / 2);
    cairo_arc(cr, x + r, y + h - r, r, M_PI / 2, M_PI);
    cairo_arc(cr, x + r, y + r, r, M_PI, 3 * M_PI / 2);
    cairo_close_path(cr);
}

enum Align { Left, Center, Right };

struct Rgb { double r, g, b; };
constexpr Rgb kFriend{1.0, 0.84, 0.25};   // フレンドの名前（黄色）
constexpr Rgb kSelf{0.45, 0.86, 1.0};     // 自分の名前（水色）

// 文字を描く。y は文字の縦の中心。戻り値は描いた幅（px）
inline int text(cairo_t* cr, const std::string& font, const std::string& str, const char* weight, int size, double x,
                double y, int width, Rgb c, Align align = Left) {
    PangoLayout* layout = pango_cairo_create_layout(cr);
    std::string desc = font + " " + weight + " " + std::to_string(size);
    PangoFontDescription* fd = pango_font_description_from_string(desc.c_str());
    pango_layout_set_font_description(layout, fd);
    if (width > 0) {
        pango_layout_set_width(layout, width * PANGO_SCALE);
        pango_layout_set_ellipsize(layout, PANGO_ELLIPSIZE_END);
        pango_layout_set_alignment(layout, align == Center ? PANGO_ALIGN_CENTER : align == Right ? PANGO_ALIGN_RIGHT : PANGO_ALIGN_LEFT);
    }
    pango_layout_set_single_paragraph_mode(layout, TRUE);
    pango_layout_set_text(layout, str.c_str(), -1);
    int lw, lh;
    pango_layout_get_pixel_size(layout, &lw, &lh);
    double tx = x;
    if (width <= 0 && align == Center) tx = x - lw / 2.0;
    if (width <= 0 && align == Right) tx = x - lw;
    cairo_set_source_rgb(cr, c.r, c.g, c.b);
    cairo_move_to(cr, tx, y - lh / 2.0);
    pango_cairo_show_layout(cr, layout);
    pango_font_description_free(fd);
    g_object_unref(layout);
    return lw;
}

// cairo の ARGB32（乗算済み）を、OpenVR に渡す非乗算の RGBA にする
inline std::vector<uint8_t> to_rgba(cairo_surface_t* s) {
    cairo_surface_flush(s);
    int w = cairo_image_surface_get_width(s), h = cairo_image_surface_get_height(s);
    int stride = cairo_image_surface_get_stride(s);
    const uint8_t* src = cairo_image_surface_get_data(s);
    std::vector<uint8_t> rgba(size_t(w) * h * 4);
    for (int y = 0; y < h; y++) {
        for (int x = 0; x < w; x++) {
            uint32_t px;
            memcpy(&px, src + y * stride + x * 4, 4);
            uint8_t a = px >> 24, r = (px >> 16) & 0xff, g = (px >> 8) & 0xff, b = px & 0xff;
            uint8_t* d = &rgba[(size_t(y) * w + x) * 4];
            if (a) {
                d[0] = uint8_t(std::min(255, r * 255 / a));
                d[1] = uint8_t(std::min(255, g * 255 / a));
                d[2] = uint8_t(std::min(255, b * 255 / a));
            } else {
                d[0] = d[1] = d[2] = 0;
            }
            d[3] = a;
        }
    }
    return rgba;
}

}  // namespace draw
