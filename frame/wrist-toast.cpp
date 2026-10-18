// SPDX-License-Identifier: MIT
// wrist-toast: Steam Frame の SteamVR に小さな通知（トースト）を出す。
// LAN から UDP で OSC の /wrist/notify を受け取り、数秒だけ視界（または手首）にカードを表示する。
// ユーザー権限で動く。描画は cairo + pango、表示は OpenVR のオーバーレイ（SetOverlayRaw）。

#include "openvr.h"

#include <arpa/inet.h>
#include <cairo.h>
#include <netinet/in.h>
#include <pango/pangocairo.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <unistd.h>

#include <cctype>
#include <cerrno>
#include <cstdarg>
#include <chrono>
#include <cmath>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <fstream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

static const char* kVersion = "0.1.0";
static volatile sig_atomic_t g_stop = 0;

static void logf(const char* fmt, ...) __attribute__((format(printf, 1, 2)));
static void logf(const char* fmt, ...) {
    char ts[16];
    time_t t = time(nullptr);
    strftime(ts, sizeof ts, "%H:%M:%S", localtime(&t));
    fprintf(stdout, "%s ", ts);
    va_list ap;
    va_start(ap, fmt);
    vfprintf(stdout, fmt, ap);
    va_end(ap);
    fputc('\n', stdout);
    fflush(stdout);
}

// ------------------------------------------------------------------ 設定（key = value の ini）
struct Config {
    int listen_port = 39570;
    std::string listen_address = "0.0.0.0";
    std::string allow_from = "lan";   // lan / any / IP をカンマ区切り
    std::string anchor = "view";      // view / left_wrist / right_wrist
    double duration_s = 4.0;
    double width_m = 0.30;
    double distance_m = 0.8;
    double offset_y_m = -0.18;
    int max_queue = 8;
    std::string font = "Noto Sans CJK JP";
};

static std::string trim(const std::string& s) {
    size_t a = s.find_first_not_of(" \t\r\n"), b = s.find_last_not_of(" \t\r\n");
    return a == std::string::npos ? "" : s.substr(a, b - a + 1);
}

static bool load_config(const std::string& path, Config& c) {
    std::ifstream f(path);
    if (!f) return false;
    std::string line;
    while (std::getline(f, line)) {
        line = trim(line);
        if (line.empty() || line[0] == '#' || line[0] == ';') continue;
        size_t eq = line.find('=');
        if (eq == std::string::npos) continue;
        std::string k = trim(line.substr(0, eq)), v = trim(line.substr(eq + 1));
        try {
            if (k == "listen_port") c.listen_port = std::stoi(v);
            else if (k == "listen_address") c.listen_address = v;
            else if (k == "allow_from") c.allow_from = v;
            else if (k == "anchor") c.anchor = v;
            else if (k == "duration_s") c.duration_s = std::stod(v);
            else if (k == "width_m") c.width_m = std::stod(v);
            else if (k == "distance_m") c.distance_m = std::stod(v);
            else if (k == "offset_y_m") c.offset_y_m = std::stod(v);
            else if (k == "max_queue") c.max_queue = std::stoi(v);
            else if (k == "font") c.font = v;
            else logf("警告: 知らない設定項目 %s は無視します", k.c_str());
        } catch (...) {
            logf("警告: %s の値 %s が読めません", k.c_str(), v.c_str());
        }
    }
    return true;
}

// ------------------------------------------------------------------ OSC（受信だけ）
struct OscMessage {
    std::string address;
    std::vector<std::string> strings;   // s の引数だけ（i / f は文字にする）
};

static bool osc_read_string(const std::string& buf, size_t& i, std::string& out) {
    size_t end = buf.find('\0', i);
    if (end == std::string::npos) return false;
    out = buf.substr(i, end - i);
    i = (end + 4) & ~size_t(3);
    return i <= buf.size();
}

static bool osc_parse(const std::string& buf, OscMessage& m) {
    size_t i = 0;
    std::string tags;
    if (buf.empty() || buf[0] != '/') return false;
    if (!osc_read_string(buf, i, m.address)) return false;
    if (!osc_read_string(buf, i, tags) || tags.empty() || tags[0] != ',') return false;
    for (size_t t = 1; t < tags.size(); t++) {
        char c = tags[t];
        if (c == 's') {
            std::string s;
            if (!osc_read_string(buf, i, s)) return false;
            m.strings.push_back(s);
        } else if (c == 'i' || c == 'f') {
            if (i + 4 > buf.size()) return false;
            uint32_t v;
            memcpy(&v, buf.data() + i, 4);
            v = ntohl(v);
            i += 4;
            if (c == 'i') m.strings.push_back(std::to_string(int32_t(v)));
            else { float f; memcpy(&f, &v, 4); m.strings.push_back(std::to_string(f)); }
        } else if (c == 'T' || c == 'F') {
            m.strings.push_back(c == 'T' ? "true" : "false");
        } else {
            return false;
        }
    }
    return true;
}

// ------------------------------------------------------------------ 受け付ける送り元
static bool is_lan(uint32_t ip_host) {
    uint8_t a = ip_host >> 24, b = (ip_host >> 16) & 0xff;
    return a == 10 || a == 127 || (a == 172 && b >= 16 && b <= 31) || (a == 192 && b == 168) || (a == 169 && b == 254);
}

static bool allowed(const std::string& rule, uint32_t ip_host) {
    if (rule == "any") return true;
    if (rule == "lan" || rule.empty()) return is_lan(ip_host);
    std::stringstream ss(rule);
    std::string item;
    while (std::getline(ss, item, ',')) {
        item = trim(item);
        if (item == "lan" && is_lan(ip_host)) return true;
        in_addr a{};
        if (inet_pton(AF_INET, item.c_str(), &a) == 1 && ntohl(a.s_addr) == ip_host) return true;
    }
    return false;
}

// 制御文字を落として、長すぎる文字列を切る（UTF-8 の途中で切らない）
static std::string clean(const std::string& s, size_t max_bytes) {
    std::string out;
    for (unsigned char c : s) {
        if (c < 0x20 && c != '\t') continue;
        out.push_back(char(c));
    }
    if (out.size() > max_bytes) {
        size_t n = max_bytes;
        while (n > 0 && (static_cast<unsigned char>(out[n]) & 0xC0) == 0x80) n--;
        out = out.substr(0, n) + "…";
    }
    return out;
}

// ------------------------------------------------------------------ 描画
struct Toast {
    std::string kind, title, text;
};

static const char* kind_icon(const std::string& kind) {
    if (kind == "join") return "➕";
    if (kind == "leave") return "➖";
    if (kind == "world") return "🌐";
    if (kind == "invite" || kind == "requestInvite") return "✉";
    if (kind == "friendRequest") return "★";
    return "●";
}

static void kind_color(const std::string& kind, double& r, double& g, double& b) {
    r = 0.35; g = 0.55; b = 0.95;                                  // その他: 青
    if (kind == "join") { r = 0.30; g = 0.80; b = 0.45; }          // 緑
    else if (kind == "leave") { r = 0.60; g = 0.60; b = 0.65; }    // 灰
    else if (kind == "world") { r = 0.30; g = 0.70; b = 0.95; }
    else if (kind == "invite" || kind == "requestInvite" || kind == "friendRequest") { r = 0.98; g = 0.70; b = 0.25; }
}

static const int kW = 640, kH = 160;

// RGBA（乗算済みでない）の 640x160 を作る
static std::vector<uint8_t> render(const Toast& t, const std::string& font, cairo_surface_t** keep = nullptr) {
    cairo_surface_t* s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, kW, kH);
    cairo_t* cr = cairo_create(s);
    cairo_set_operator(cr, CAIRO_OPERATOR_CLEAR);
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);

    // 角の丸いカード
    double rad = 28, x = 4, y = 4, w = kW - 8, h = kH - 8;
    cairo_new_sub_path(cr);
    cairo_arc(cr, x + w - rad, y + rad, rad, -M_PI / 2, 0);
    cairo_arc(cr, x + w - rad, y + h - rad, rad, 0, M_PI / 2);
    cairo_arc(cr, x + rad, y + h - rad, rad, M_PI / 2, M_PI);
    cairo_arc(cr, x + rad, y + rad, rad, M_PI, 3 * M_PI / 2);
    cairo_close_path(cr);
    cairo_set_source_rgba(cr, 0.08, 0.09, 0.11, 0.90);
    cairo_fill_preserve(cr);
    double r, g, b;
    kind_color(t.kind, r, g, b);
    cairo_set_source_rgba(cr, r, g, b, 0.9);
    cairo_set_line_width(cr, 4);
    cairo_stroke(cr);

    // アイコンの丸
    cairo_arc(cr, 76, kH / 2.0, 46, 0, 2 * M_PI);
    cairo_set_source_rgba(cr, r, g, b, 0.95);
    cairo_fill(cr);

    auto draw_text = [&](const std::string& str, const char* weight, int size, double tx, double ty, int width,
                         double cr_, double cg, double cb, bool center) {
        PangoLayout* layout = pango_cairo_create_layout(cr);
        std::string desc = font + " " + weight + " " + std::to_string(size);
        PangoFontDescription* fd = pango_font_description_from_string(desc.c_str());
        pango_layout_set_font_description(layout, fd);
        pango_layout_set_width(layout, width * PANGO_SCALE);
        pango_layout_set_ellipsize(layout, PANGO_ELLIPSIZE_END);
        if (center) pango_layout_set_alignment(layout, PANGO_ALIGN_CENTER);
        pango_layout_set_text(layout, str.c_str(), -1);
        int lw, lh;
        pango_layout_get_pixel_size(layout, &lw, &lh);
        cairo_set_source_rgb(cr, cr_, cg, cb);
        cairo_move_to(cr, tx, center ? ty - lh / 2.0 : ty);
        pango_cairo_show_layout(cr, layout);
        pango_font_description_free(fd);
        g_object_unref(layout);
    };
    draw_text(kind_icon(t.kind), "Bold", 34, 30, kH / 2.0, 92, 0.08, 0.09, 0.11, true);
    draw_text(t.title, "Bold", 26, 142, 28, kW - 170, 1, 1, 1, false);
    draw_text(t.text, "Regular", 22, 142, 84, kW - 170, 0.82, 0.84, 0.88, false);

    cairo_surface_flush(s);
    std::vector<uint8_t> rgba(kW * kH * 4);
    const uint8_t* src = cairo_image_surface_get_data(s);
    int stride = cairo_image_surface_get_stride(s);
    for (int yy = 0; yy < kH; yy++) {
        for (int xx = 0; xx < kW; xx++) {
            uint32_t px;
            memcpy(&px, src + yy * stride + xx * 4, 4);   // ARGB（乗算済み）
            uint8_t a = px >> 24, rr = (px >> 16) & 0xff, gg = (px >> 8) & 0xff, bb = px & 0xff;
            uint8_t* d = &rgba[(yy * kW + xx) * 4];
            if (a) {
                d[0] = uint8_t(std::min(255, rr * 255 / a));
                d[1] = uint8_t(std::min(255, gg * 255 / a));
                d[2] = uint8_t(std::min(255, bb * 255 / a));
            } else {
                d[0] = d[1] = d[2] = 0;
            }
            d[3] = a;
        }
    }
    cairo_destroy(cr);
    if (keep) *keep = s;
    else cairo_surface_destroy(s);
    return rgba;
}

// ------------------------------------------------------------------ SteamVR
struct Vr {
    bool up = false;
    vr::VROverlayHandle_t overlay = vr::k_ulOverlayHandleInvalid;
};

static bool vr_start(Vr& v, const Config& c) {
    vr::EVRInitError err = vr::VRInitError_None;
    vr::VR_Init(&err, vr::VRApplication_Overlay);
    if (err != vr::VRInitError_None) return false;
    vr::EVROverlayError oe = vr::VROverlay()->CreateOverlay("cympfh.vrc-wrist-notifier", "Wrist Notifier", &v.overlay);
    if (oe != vr::VROverlayError_None) {
        logf("オーバーレイを作れません: %s", vr::VROverlay()->GetOverlayErrorNameFromEnum(oe));
        vr::VR_Shutdown();
        return false;
    }
    vr::VROverlay()->SetOverlayWidthInMeters(v.overlay, float(c.width_m));
    vr::VROverlay()->SetOverlaySortOrder(v.overlay, 100);
    v.up = true;
    logf("SteamVR につながりました");
    return true;
}

static void vr_stop(Vr& v) {
    if (!v.up) return;
    vr::VR_Shutdown();
    v.up = false;
    v.overlay = vr::k_ulOverlayHandleInvalid;
}

static void place(Vr& v, const Config& c) {
    vr::HmdMatrix34_t m{};
    vr::TrackedDeviceIndex_t dev = vr::k_unTrackedDeviceIndex_Hmd;
    bool wrist = c.anchor == "left_wrist" || c.anchor == "right_wrist";
    if (wrist) {
        dev = vr::VRSystem()->GetTrackedDeviceIndexForControllerRole(
            c.anchor == "left_wrist" ? vr::TrackedControllerRole_LeftHand : vr::TrackedControllerRole_RightHand);
        if (dev == vr::k_unTrackedDeviceIndexInvalid) {   // コントローラーが無ければ視界に出す
            dev = vr::k_unTrackedDeviceIndex_Hmd;
            wrist = false;
        }
    }
    if (wrist) {
        // 手首の上、少し手前に傾けて置く（X 軸まわりに -60°）
        double a = -60.0 * M_PI / 180.0;
        m.m[0][0] = 1;
        m.m[1][1] = float(cos(a)); m.m[1][2] = float(-sin(a));
        m.m[2][1] = float(sin(a)); m.m[2][2] = float(cos(a));
        m.m[0][3] = 0; m.m[1][3] = 0.06f; m.m[2][3] = 0.12f;
    } else {
        m.m[0][0] = m.m[1][1] = m.m[2][2] = 1;
        m.m[0][3] = 0; m.m[1][3] = float(c.offset_y_m); m.m[2][3] = float(-c.distance_m);
    }
    vr::VROverlay()->SetOverlayTransformTrackedDeviceRelative(v.overlay, dev, &m);
}

// ------------------------------------------------------------------ 自己テスト
static int self_test() {
    int fails = 0;
    auto check = [&](bool ok, const char* what) {
        printf("%s %s\n", ok ? "OK  " : "FAIL", what);
        if (!ok) fails++;
    };
    auto pad = [](std::string s) { s.push_back('\0'); while (s.size() % 4) s.push_back('\0'); return s; };
    std::string msg = pad("/wrist/notify") + pad(",sss") + pad("join") + pad("参加") + pad("Alice が来ました");
    OscMessage m;
    check(osc_parse(msg, m) && m.address == "/wrist/notify" && m.strings.size() == 3 && m.strings[2] == "Alice が来ました",
          "OSC ,sss を読める");
    OscMessage bad;
    check(!osc_parse("garbage", bad), "OSC でないものは捨てる");
    check(!osc_parse(pad("/x") + pad(",s"), bad), "途中で切れた OSC は捨てる");
    check(allowed("lan", 0xC0A80B18) && allowed("lan", 0x0A000001) && !allowed("lan", 0x08080808), "lan は LAN 内だけ");
    check(allowed("192.0.2.5", 0xC0000205) && !allowed("192.0.2.5", 0xC0000206), "IP の指定");
    check(clean(std::string("a\nb"), 10) == "ab", "制御文字を落とす");
    std::string longjp;
    for (int i = 0; i < 100; i++) longjp += "あ";
    std::string cut = clean(longjp, 31);
    check(cut.size() <= 31 + 3 && cut.substr(cut.size() - 3) == "…", "UTF-8 の途中で切らない");
    printf("%s\n", fails ? "自己テスト失敗" : "自己テスト OK");
    return fails ? 1 : 0;
}

// ------------------------------------------------------------------ 本体
static std::string default_config_path() {
    const char* x = getenv("XDG_CONFIG_HOME");
    std::string base = x && *x ? x : std::string(getenv("HOME") ? getenv("HOME") : ".") + "/.config";
    return base + "/vrc-wrist-notifier/config.ini";
}

static double now_s() {
    using namespace std::chrono;
    return duration<double>(steady_clock::now().time_since_epoch()).count();
}

int main(int argc, char** argv) {
    std::string config_path = default_config_path(), dump_png, title = "テスト", text = "wrist-toast の表示テスト",
                kind = "test";
    for (int i = 1; i < argc; i++) {
        std::string a = argv[i];
        auto next = [&]() { return i + 1 < argc ? std::string(argv[++i]) : std::string(); };
        if (a == "--version") { printf("wrist-toast %s\n", kVersion); return 0; }
        if (a == "--self-test") return self_test();
        if (a == "--config") config_path = next();
        else if (a == "--dump-png") dump_png = next();
        else if (a == "--title") title = next();
        else if (a == "--text") text = next();
        else if (a == "--kind") kind = next();
        else {
            fprintf(stderr, "使い方: wrist-toast [--config FILE] [--self-test] [--dump-png out.png --kind join --title T --text S] [--version]\n");
            return 2;
        }
    }
    Config c;
    bool have = load_config(config_path, c);

    if (!dump_png.empty()) {   // SteamVR なしで、カードを PNG に書き出す（見た目の確認用）
        cairo_surface_t* s = nullptr;
        render({kind, title, text}, c.font, &s);
        cairo_status_t st = cairo_surface_write_to_png(s, dump_png.c_str());
        cairo_surface_destroy(s);
        printf("%s\n", st == CAIRO_STATUS_SUCCESS ? dump_png.c_str() : "PNG を書けませんでした");
        return st == CAIRO_STATUS_SUCCESS ? 0 : 1;
    }

    signal(SIGTERM, [](int) { g_stop = 1; });
    signal(SIGINT, [](int) { g_stop = 1; });

    int sock = socket(AF_INET, SOCK_DGRAM, 0);
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(uint16_t(c.listen_port));
    if (inet_pton(AF_INET, c.listen_address.c_str(), &addr.sin_addr) != 1 ||
        bind(sock, reinterpret_cast<sockaddr*>(&addr), sizeof addr) != 0) {
        logf("UDP %s:%d で待てません: %s", c.listen_address.c_str(), c.listen_port, strerror(errno));
        return 1;
    }
    logf("wrist-toast %s: UDP %s:%d で待っています（受け付ける送り元: %s、表示場所: %s、設定: %s）", kVersion,
         c.listen_address.c_str(), c.listen_port, c.allow_from.c_str(), c.anchor.c_str(),
         have ? config_path.c_str() : "既定値");

    Vr v;
    std::deque<Toast> queue;
    bool showing = false;
    double shown_at = 0, next_vr_try = 0, window_start = 0;
    int window_count = 0;
    long received = 0, shown = 0, dropped = 0;

    while (!g_stop) {
        double now = now_s();
        if (!v.up && now >= next_vr_try) {
            if (!vr_start(v, c)) next_vr_try = now + 5;
        }
        if (v.up) {
            vr::VREvent_t ev;
            while (vr::VRSystem()->PollNextEvent(&ev, sizeof ev)) {
                if (ev.eventType == vr::VREvent_Quit) {
                    logf("SteamVR が終わるので切断します");
                    vr::VRSystem()->AcknowledgeQuit_Exiting();
                    vr_stop(v);
                    showing = false;
                    next_vr_try = now + 5;
                    break;
                }
            }
        }

        fd_set fds;
        FD_ZERO(&fds);
        FD_SET(sock, &fds);
        timeval tv{0, 50000};
        if (select(sock + 1, &fds, nullptr, nullptr, &tv) > 0) {
            char buf[2048];
            sockaddr_in from{};
            socklen_t fl = sizeof from;
            ssize_t n = recvfrom(sock, buf, sizeof buf, 0, reinterpret_cast<sockaddr*>(&from), &fl);
            uint32_t ip = ntohl(from.sin_addr.s_addr);
            char ipstr[INET_ADDRSTRLEN];
            inet_ntop(AF_INET, &from.sin_addr, ipstr, sizeof ipstr);
            OscMessage m;
            if (n <= 0) {
            } else if (!allowed(c.allow_from, ip)) {
                logf("受け付けない送り元 %s からのパケットを捨てました", ipstr);
            } else if (!osc_parse(std::string(buf, size_t(n)), m) || m.address != "/wrist/notify" || m.strings.empty()) {
                logf("%s から /wrist/notify 以外を受け取ったので捨てました", ipstr);
            } else {
                received++;
                if (now - window_start > 1.0) { window_start = now; window_count = 0; }
                Toast t;
                if (m.strings.size() >= 3) t = {m.strings[0], m.strings[1], m.strings[2]};
                else if (m.strings.size() == 2) t = {"info", m.strings[0], m.strings[1]};
                else t = {"info", "VRChat", m.strings[0]};
                t.kind = clean(t.kind, 24);
                t.title = clean(t.title, 120);
                t.text = clean(t.text, 240);
                if (++window_count > 10 || int(queue.size()) >= c.max_queue) {
                    dropped++;
                    logf("多すぎるので捨てました: [%s] %s", t.kind.c_str(), t.title.c_str());
                } else {
                    logf("受信 %s: [%s] %s / %s", ipstr, t.kind.c_str(), t.title.c_str(), t.text.c_str());
                    queue.push_back(t);
                }
            }
        }

        if (v.up) {
            if (showing) {
                double age = now - shown_at, fade = 0.4;
                if (age >= c.duration_s) {
                    vr::VROverlay()->HideOverlay(v.overlay);
                    showing = false;
                } else {
                    double alpha = age < fade ? age / fade : (c.duration_s - age < fade ? (c.duration_s - age) / fade : 1.0);
                    vr::VROverlay()->SetOverlayAlpha(v.overlay, float(alpha));
                }
            }
            if (!showing && !queue.empty()) {
                Toast t = queue.front();
                queue.pop_front();
                std::vector<uint8_t> px = render(t, c.font);
                vr::EVROverlayError e = vr::VROverlay()->SetOverlayRaw(v.overlay, px.data(), kW, kH, 4);
                if (e != vr::VROverlayError_None) {
                    logf("表示できませんでした: %s", vr::VROverlay()->GetOverlayErrorNameFromEnum(e));
                } else {
                    place(v, c);
                    vr::VROverlay()->SetOverlayAlpha(v.overlay, 0.0f);
                    vr::VROverlay()->ShowOverlay(v.overlay);
                    showing = true;
                    shown_at = now;
                    shown++;
                    logf("表示: [%s] %s", t.kind.c_str(), t.title.c_str());
                }
            }
        } else if (!queue.empty()) {
            // SteamVR が動いていないあいだの通知は、あとでまとめて出しても古いので捨てる
            logf("SteamVR が動いていないので表示しません: %zu 件", queue.size());
            dropped += long(queue.size());
            queue.clear();
        }
    }
    vr_stop(v);
    close(sock);
    logf("終了: 受信 %ld / 表示 %ld / 捨てた %ld", received, shown, dropped);
    return 0;
}
