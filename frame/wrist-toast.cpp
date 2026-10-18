// SPDX-License-Identifier: MIT
// wrist-toast: Steam Frame の SteamVR に小さな通知（トースト）を出し、入退室の履歴をダッシュボードのパネルで見せる。
// LAN から UDP で OSC の /wrist/notify（通知）と /wrist/history（過去の分の読み込み）を受け取る。
// ユーザー権限で動く。描画は cairo + pango、表示は OpenVR のオーバーレイ（Vulkan のテクスチャ。だめなら SetOverlayRaw）。

#include "openvr.h"
#include "draw.h"
#include "history.h"
#include "panel.h"
#include "vk_texture.h"

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

static const char* kVersion = "0.2.1";
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
    std::string history_dir;          // 空なら ~/.local/state/vrc-wrist-notifier
    bool history_panel = true;        // ダッシュボードに履歴のパネルを出す
    double panel_width_m = 2.6;
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
            else if (k == "history_dir") c.history_dir = v;
            else if (k == "history_panel") c.history_panel = (v == "true" || v == "1" || v == "yes");
            else if (k == "panel_width_m") c.panel_width_m = std::stod(v);
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

// 種類は「基本.詳細」でもよい（例: alert.temp、care.break）。色は基本で、アイコンは詳細で決める。
// VRChat 以外の通知（frame-care など）: alert / urgent = 赤、warn = 琥珀、care / info = 青緑
static std::string kind_base(const std::string& kind) { return kind.substr(0, kind.find('.')); }
static std::string kind_sub(const std::string& kind) {
    size_t d = kind.find('.');
    return d == std::string::npos ? "" : kind.substr(d + 1);
}
static bool is_alert(const std::string& kind) {
    std::string b = kind_base(kind);
    return b == "alert" || b == "urgent";
}

static const char* kind_icon(const std::string& kind) {
    if (kind == "join") return "➕";
    if (kind == "leave") return "➖";
    if (kind == "world") return "🌐";
    if (kind == "invite" || kind == "requestInvite") return "✉";
    if (kind == "friendRequest") return "★";
    std::string sub = kind_sub(kind), base = kind_base(kind);
    if (sub == "temp") return "🌡";
    if (sub == "battery") return "🔋";
    if (sub == "charge") return "⚡";
    if (sub == "break") return "☕";
    if (sub == "water") return "💧";
    if (is_alert(kind)) return "!";
    if (base == "warn") return "⚠";
    if (base == "care" || base == "info") return "♥";
    return "●";
}

static void kind_color(const std::string& kind, double& r, double& g, double& b) {
    std::string base = kind_base(kind);
    r = 0.35; g = 0.55; b = 0.95;                                  // その他: 青
    if (kind == "join") { r = 0.30; g = 0.80; b = 0.45; }          // 緑
    else if (kind == "leave") { r = 0.60; g = 0.60; b = 0.65; }    // 灰
    else if (kind == "world") { r = 0.30; g = 0.70; b = 0.95; }
    else if (kind == "invite" || kind == "requestInvite" || kind == "friendRequest") { r = 0.98; g = 0.70; b = 0.25; }
    else if (is_alert(kind)) { r = 0.93; g = 0.27; b = 0.27; }    // 赤
    else if (base == "warn") { r = 1.00; g = 0.58; b = 0.10; }    // 琥珀
    else if (base == "care" || base == "info") { r = 0.18; g = 0.76; b = 0.70; }   // 青緑
}

static const int kW = 640, kH = 160;

// トーストのカード（640x160）
static cairo_surface_t* render_toast_surface(const Toast& t, const std::string& font) {
    cairo_surface_t* s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, kW, kH);
    cairo_t* cr = cairo_create(s);
    bool alert = is_alert(t.kind);
    draw::rounded(cr, 4, 4, kW - 8, kH - 8, 28);
    if (alert) cairo_set_source_rgba(cr, 0.22, 0.06, 0.07, 0.94);   // 急ぎは背景も赤み
    else cairo_set_source_rgba(cr, 0.08, 0.09, 0.11, 0.90);
    cairo_fill_preserve(cr);
    double r, g, b;
    kind_color(t.kind, r, g, b);
    cairo_set_source_rgba(cr, r, g, b, 0.9);
    cairo_set_line_width(cr, alert ? 8 : 4);
    cairo_stroke(cr);
    cairo_arc(cr, 76, kH / 2.0, 46, 0, 2 * M_PI);
    cairo_set_source_rgba(cr, r, g, b, 0.95);
    cairo_fill(cr);
    draw::text(cr, font, kind_icon(t.kind), "Bold", 34, 76, kH / 2.0, 0, {0.08, 0.09, 0.11}, draw::Center);
    draw::text(cr, font, t.title, "Bold", 26, 142, 50, kW - 170, {1, 1, 1});
    draw::text(cr, font, t.text, "Regular", 22, 142, 104, kW - 170, {0.82, 0.84, 0.88});
    cairo_destroy(cr);
    return s;
}

// ------------------------------------------------------------------ SteamVR
static const char* kToastKey = "cympfh.vrc-wrist-notifier";
static const char* kPanelKey = "cympfh.vrc-wrist-notifier.history";

struct Vr {
    bool up = false;
    vr::VROverlayHandle_t toast = vr::k_ulOverlayHandleInvalid;
    vr::VROverlayHandle_t panel = vr::k_ulOverlayHandleInvalid;
    vr::VROverlayHandle_t thumb = vr::k_ulOverlayHandleInvalid;
    VulkanContext* vk = nullptr;
    OverlayTexture* toast_tex = nullptr;
    OverlayTexture* panel_tex = nullptr;
    OverlayTexture* thumb_tex = nullptr;
    bool use_vulkan = false;
};

static const char* oerr(vr::EVROverlayError e) { return vr::VROverlay()->GetOverlayErrorNameFromEnum(e); }

// 画像をオーバーレイに渡す（Vulkan が使えればテクスチャ、だめなら SetOverlayRaw）
static bool upload(Vr& v, vr::VROverlayHandle_t h, OverlayTexture*& tex, const std::vector<uint8_t>& rgba, int w, int hgt) {
    if (v.use_vulkan) {
        std::string err;
        if (!tex) {
            tex = new OverlayTexture();
            if (!tex->create(*v.vk, w, hgt, err)) {
                logf("Vulkan の画像を作れません（%s）。SetOverlayRaw に切り替えます", err.c_str());
                delete tex;
                tex = nullptr;
                v.use_vulkan = false;
            }
        }
        if (tex) {
            if (tex->update(h, rgba.data(), err)) return true;
            logf("画像を渡せませんでした: %s", err.c_str());
            return false;
        }
    }
    vr::EVROverlayError e = vr::VROverlay()->SetOverlayRaw(h, const_cast<uint8_t*>(rgba.data()), uint32_t(w), uint32_t(hgt), 4);
    if (e != vr::VROverlayError_None) logf("SetOverlayRaw: %s", oerr(e));
    return e == vr::VROverlayError_None;
}

static bool create_panel(Vr& v, const Config& c) {
    vr::EVROverlayError e = vr::VROverlay()->CreateDashboardOverlay(kPanelKey, "入退室の履歴", &v.panel, &v.thumb);
    if (e != vr::VROverlayError_None) {
        logf("ダッシュボードのパネルを作れません: %s", oerr(e));
        v.panel = v.thumb = vr::k_ulOverlayHandleInvalid;
        return false;
    }
    vr::VROverlay()->SetOverlayWidthInMeters(v.panel, float(c.panel_width_m));
    vr::VROverlay()->SetOverlayInputMethod(v.panel, vr::VROverlayInputMethod_Mouse);
    vr::HmdVector2_t scale = {{float(HistoryPanel::kW), float(HistoryPanel::kH)}};
    vr::VROverlay()->SetOverlayMouseScale(v.panel, &scale);
    vr::VROverlay()->SetOverlayFlag(v.panel, vr::VROverlayFlags_SendVRSmoothScrollEvents, true);
    logf("ダッシュボードに「入退室の履歴」を登録しました");
    return true;
}

static void vr_stop(Vr& v) {
    if (!v.up) return;
    vr::VR_Shutdown();
    // Vulkan のものは VR_Shutdown のあとで壊す
    delete v.toast_tex; delete v.panel_tex; delete v.thumb_tex;
    v.toast_tex = v.panel_tex = v.thumb_tex = nullptr;
    delete v.vk;
    v.vk = nullptr;
    v.up = false;
    v.toast = v.panel = v.thumb = vr::k_ulOverlayHandleInvalid;
}

static bool vr_start(Vr& v, const Config& c) {
    vr::EVRInitError err = vr::VRInitError_None;
    vr::VR_Init(&err, vr::VRApplication_Overlay);
    if (err != vr::VRInitError_None) return false;
    vr::EVROverlayError oe = vr::VROverlay()->CreateOverlay(kToastKey, "Wrist Notifier", &v.toast);
    if (oe != vr::VROverlayError_None) {
        logf("オーバーレイを作れません: %s", oerr(oe));
        vr::VR_Shutdown();
        return false;
    }
    v.up = true;
    vr::VROverlay()->SetOverlayWidthInMeters(v.toast, float(c.width_m));
    vr::VROverlay()->SetOverlaySortOrder(v.toast, 100);
    v.vk = new VulkanContext();
    std::string verr;
    v.use_vulkan = v.vk->init(verr);
    if (!v.use_vulkan) logf("Vulkan を使えません（%s）。SetOverlayRaw で表示します", verr.c_str());
    if (c.history_panel) create_panel(v, c);
    logf("SteamVR につながりました（画像の渡し方: %s）", v.use_vulkan ? "Vulkan" : "SetOverlayRaw");
    return true;
}

static void place(Vr& v, const Config& c) {
    vr::HmdMatrix34_t m{};
    vr::TrackedDeviceIndex_t dev = vr::k_unTrackedDeviceIndex_Hmd;
    bool wrist = c.anchor == "left_wrist" || c.anchor == "right_wrist";
    if (wrist) {
        dev = vr::VRSystem()->GetTrackedDeviceIndexForControllerRole(
            c.anchor == "left_wrist" ? vr::TrackedControllerRole_LeftHand : vr::TrackedControllerRole_RightHand);
        if (dev == vr::k_unTrackedDeviceIndexInvalid) {
            dev = vr::k_unTrackedDeviceIndex_Hmd;
            wrist = false;
        }
    }
    if (wrist) {
        double a = -60.0 * M_PI / 180.0;
        m.m[0][0] = 1;
        m.m[1][1] = float(cos(a)); m.m[1][2] = float(-sin(a));
        m.m[2][1] = float(sin(a)); m.m[2][2] = float(cos(a));
        m.m[0][3] = 0; m.m[1][3] = 0.06f; m.m[2][3] = 0.12f;
    } else {
        m.m[0][0] = m.m[1][1] = m.m[2][2] = 1;
        m.m[0][3] = 0; m.m[1][3] = float(c.offset_y_m); m.m[2][3] = float(-c.distance_m);
    }
    vr::VROverlay()->SetOverlayTransformTrackedDeviceRelative(v.toast, dev, &m);
}

// 別のプロセスから、ダッシュボードのパネルが SteamVR にあるかだけ確かめる（Background でつなぐ）
static int probe() {
    vr::EVRInitError err = vr::VRInitError_None;
    vr::VR_Init(&err, vr::VRApplication_Background);
    if (err != vr::VRInitError_None) {
        printf("SteamVR につながりません: %s\n", vr::VR_GetVRInitErrorAsEnglishDescription(err));
        return 1;
    }
    int rc = 0;
    for (const char* key : {kPanelKey, kToastKey}) {
        vr::VROverlayHandle_t h = vr::k_ulOverlayHandleInvalid;
        vr::EVROverlayError e = vr::VROverlay()->FindOverlay(key, &h);
        uint32_t w = 0, hh = 0;
        if (e == vr::VROverlayError_None) vr::VROverlay()->GetOverlayTextureSize(h, &w, &hh);
        printf("FindOverlay(%s) -> %s、画像 %ux%u\n", key, oerr(e), w, hh);
        if (e != vr::VROverlayError_None) rc = 1;
    }
    printf("ダッシュボード: %s\n", vr::VROverlay()->IsDashboardVisible() ? "開いている" : "閉じている");
    vr::VR_Shutdown();
    return rc;
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

    double cr_, cg, cb;
    kind_color("alert.temp", cr_, cg, cb);
    check(cr_ > 0.9 && cg < 0.4 && std::string(kind_icon("alert.temp")) == "🌡" && is_alert("urgent"), "alert.temp は赤で温度計");
    kind_color("warn", cr_, cg, cb);
    check(cr_ > 0.9 && cg > 0.5 && cb < 0.2 && std::string(kind_icon("warn")) == "⚠", "warn は琥珀");
    kind_color("care.break", cr_, cg, cb);
    check(cg > 0.7 && cb > 0.6 && cr_ < 0.3 && std::string(kind_icon("care.break")) == "☕", "care.break は青緑でカップ");
    kind_color("join", cr_, cg, cb);
    check(std::string(kind_icon("join")) == "➕" && cg > 0.7 && cr_ < 0.4 && !is_alert("join"), "VRChat の種類は今までどおり");

    HistEvent e{"2026-01-02 21:00:00", "join", "名前 \"引用\" \\ 😀", "ワールド", "k1"}, back;
    check(hist::from_json(hist::to_json(e), back) && back.name == e.name && back.world == e.world && back.key == "k1",
          "履歴の JSON を書いて読み戻せる");
    char tmpl[] = "/tmp/wrist-toast-test.XXXXXX";
    std::string dir = mkdtemp(tmpl);
    {
        History h(dir + "/state");
        check(h.add(e), "履歴に追加できる");
        check(!h.add(e), "同じ鍵は 2 回入れない");
        check(h.add({"2026-01-02 20:00:00", "world", "前のワールド", "前のワールド", "k0"}), "過去の分を追加できる");
        check(!h.add({"2026-01-02 20:00:00", "invite", "x", "", "k9"}), "参加・退出・ワールド以外は入れない");
        check(h.events().size() == 2 && h.events().front().key == "k0", "時刻の順に並ぶ");
        History h2(dir + "/state");
        h2.load();
        check(h2.events().size() == 2 && !h2.add(e), "読み直しても重複しない");
        printf("（テスト用の一時ファイル: %s。消していません）\n", dir.c_str());
    }
    printf("%s\n", fails ? "自己テスト失敗" : "自己テスト OK");
    return fails ? 1 : 0;
}

// ------------------------------------------------------------------ 本体
static std::string home() { return getenv("HOME") ? getenv("HOME") : "."; }
static std::string default_config_path() {
    const char* x = getenv("XDG_CONFIG_HOME");
    return (x && *x ? std::string(x) : home() + "/.config") + "/vrc-wrist-notifier/config.ini";
}
static std::string default_history_dir() {
    const char* x = getenv("XDG_STATE_HOME");
    return (x && *x ? std::string(x) : home() + "/.local/state") + "/vrc-wrist-notifier";
}

static double now_s() {
    using namespace std::chrono;
    return duration<double>(steady_clock::now().time_since_epoch()).count();
}

static bool write_png(cairo_surface_t* s, const std::string& path) {
    cairo_status_t st = cairo_surface_write_to_png(s, path.c_str());
    cairo_surface_destroy(s);
    printf("%s\n", st == CAIRO_STATUS_SUCCESS ? path.c_str() : "PNG を書けませんでした");
    return st == CAIRO_STATUS_SUCCESS;
}

int main(int argc, char** argv) {
    std::string config_path = default_config_path(), dump_png, dump_history, dump_thumb, history_dir_opt,
                title = "テスト", text = "wrist-toast の表示テスト", kind = "test", filter = "all";
    double scroll_px = 0;
    for (int i = 1; i < argc; i++) {
        std::string a = argv[i];
        auto next = [&]() { return i + 1 < argc ? std::string(argv[++i]) : std::string(); };
        if (a == "--version") { printf("wrist-toast %s\n", kVersion); return 0; }
        if (a == "--self-test") return self_test();
        if (a == "--probe") return probe();
        if (a == "--config") config_path = next();
        else if (a == "--dump-png") dump_png = next();
        else if (a == "--dump-history-png") dump_history = next();
        else if (a == "--dump-thumbnail-png") dump_thumb = next();
        else if (a == "--history-dir") history_dir_opt = next();
        else if (a == "--filter") filter = next();
        else if (a == "--scroll") scroll_px = std::stod(next());
        else if (a == "--title") title = next();
        else if (a == "--text") text = next();
        else if (a == "--kind") kind = next();
        else {
            fprintf(stderr,
                    "使い方: wrist-toast [--config FILE]\n"
                    "  --self-test                 自己テスト\n"
                    "  --probe                     動いている wrist-toast のパネルが SteamVR にあるか確かめる\n"
                    "  --dump-png OUT [--kind K --title T --text S]   通知のカードを PNG に\n"
                    "  --dump-history-png OUT [--history-dir D] [--filter all|join|leave] [--scroll PX]   履歴のパネルを PNG に\n"
                    "  --dump-thumbnail-png OUT    ダッシュボードのアイコンを PNG に\n"
                    "  --version\n");
            return 2;
        }
    }
    Config c;
    bool have = load_config(config_path, c);
    std::string history_dir = !history_dir_opt.empty() ? history_dir_opt : !c.history_dir.empty() ? c.history_dir : default_history_dir();

    // SteamVR なしで、見た目を PNG に書き出す
    if (!dump_png.empty()) return write_png(render_toast_surface({kind, title, text}, c.font), dump_png) ? 0 : 1;
    if (!dump_history.empty() || !dump_thumb.empty()) {
        HistoryPanel panel(c.font);
        cairo_surface_t* s = nullptr;
        if (!dump_thumb.empty()) {
            panel.render_thumbnail(&s);
            if (!write_png(s, dump_thumb)) return 1;
        }
        if (!dump_history.empty()) {
            History h(history_dir);
            h.load();
            panel.set_filter(filter == "join" ? HistoryPanel::JoinOnly : filter == "leave" ? HistoryPanel::LeaveOnly : HistoryPanel::All);
            panel.render(h.events());   // 中身の高さを決める
            panel.scroll(-scroll_px / 120.0);
            panel.render(h.events(), &s);
            if (!write_png(s, dump_history)) return 1;
        }
        return 0;
    }

    signal(SIGTERM, [](int) { g_stop = 1; });
    signal(SIGINT, [](int) { g_stop = 1; });

    int sock = socket(AF_INET, SOCK_DGRAM, 0);
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(uint16_t(c.listen_port));
    if (inet_pton(AF_INET, c.listen_address.c_str(), &addr.sin_addr) != 1 ||
        bind(sock, reinterpret_cast<sockaddr*>(&addr), sizeof addr) != 0) {
        logf("UDP %s:%d で待てません: %s（もう動いていませんか？）", c.listen_address.c_str(), c.listen_port, strerror(errno));
        return 1;
    }
    History history(history_dir);
    history.load();
    logf("wrist-toast %s: UDP %s:%d で待っています（受け付ける送り元: %s、表示場所: %s、設定: %s、履歴 %zu 件: %s）", kVersion,
         c.listen_address.c_str(), c.listen_port, c.allow_from.c_str(), c.anchor.c_str(),
         have ? config_path.c_str() : "既定値", history.events().size(), history.path().c_str());

    Vr v;
    HistoryPanel panel(c.font);
    std::deque<Toast> queue;
    bool showing = false, panel_dirty = true, thumb_done = false, panel_visible = false;
    unsigned panel_version = ~0u;
    double shown_at = 0, shown_dur = 4, next_vr_try = 0, window_start = 0, next_check = 0;
    int window_count = 0;
    long received = 0, shown = 0, dropped = 0, hist_added = 0, hist_logged = 0;
    std::string current_world;

    while (!g_stop) {
        double now = now_s();
        if (!v.up && now >= next_vr_try) {
            if (!vr_start(v, c)) next_vr_try = now + 5;
            else { panel_dirty = true; thumb_done = false; }
        }
        if (v.up) {
            vr::VREvent_t ev;
            bool quit = false;
            while (vr::VRSystem()->PollNextEvent(&ev, sizeof ev)) {
                if (ev.eventType == vr::VREvent_Quit) quit = true;
            }
            if (v.panel != vr::k_ulOverlayHandleInvalid) {
                while (vr::VROverlay()->PollNextOverlayEvent(v.panel, &ev, sizeof ev)) {
                    double x = ev.data.mouse.x, y = HistoryPanel::kH - ev.data.mouse.y;   // 左下原点 → 左上原点
                    switch (ev.eventType) {
                        case vr::VREvent_MouseMove: panel_dirty |= panel.pointer_move(x, y); break;
                        case vr::VREvent_MouseButtonDown:
                            if (ev.data.mouse.button == vr::VRMouseButton_Left) panel_dirty |= panel.pointer_down(x, y);
                            break;
                        case vr::VREvent_MouseButtonUp:
                            if (ev.data.mouse.button == vr::VRMouseButton_Left) panel_dirty |= panel.pointer_up(x, y);
                            break;
                        case vr::VREvent_FocusLeave: panel_dirty |= panel.pointer_leave(); break;
                        case vr::VREvent_ScrollSmooth:
                        case vr::VREvent_ScrollDiscrete: panel_dirty |= panel.scroll(ev.data.scroll.ydelta); break;
                        case vr::VREvent_OverlayShown: panel_visible = true; panel_dirty = true; logf("履歴のパネルが開きました"); break;
                        case vr::VREvent_OverlayHidden: panel_visible = false; break;
                        default: break;
                    }
                }
            }
            if (quit) {
                logf("SteamVR が終わるので切断します");
                vr::VRSystem()->AcknowledgeQuit_Exiting();
                vr_stop(v);
                showing = false;
                next_vr_try = now + 5;
            }
        }
        // パネルが SteamVR から消えていたら作り直す（vrdashboard の作り直しなど）
        if (v.up && c.history_panel && now >= next_check) {
            next_check = now + 5;
            vr::VROverlayHandle_t found = vr::k_ulOverlayHandleInvalid;
            if (vr::VROverlay()->FindOverlay(kPanelKey, &found) != vr::VROverlayError_None || found != v.panel) {
                logf("履歴のパネルが SteamVR に見つからないので作り直します");
                delete v.panel_tex; delete v.thumb_tex;
                v.panel_tex = v.thumb_tex = nullptr;
                if (v.panel != vr::k_ulOverlayHandleInvalid) vr::VROverlay()->DestroyOverlay(v.panel);
                create_panel(v, c);
                panel_dirty = true;
                thumb_done = false;
            }
        }

        fd_set fds;
        timeval tv{0, panel_visible ? 16000 : 50000};
        auto readable = [&]() {
            FD_ZERO(&fds);
            FD_SET(sock, &fds);
            bool r = select(sock + 1, &fds, nullptr, nullptr, &tv) > 0;
            tv = {0, 0};   // 2 回目からは待たずに、溜まっている分だけ読む
            return r;
        };
        while (readable()) {
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
            } else if (!osc_parse(std::string(buf, size_t(n)), m) || m.strings.empty()) {
                logf("%s から読めないパケットを受け取ったので捨てました", ipstr);
            } else if (m.address == "/wrist/history" && m.strings.size() >= 5) {
                // 過去の分（Windows 側が起動時に VRChat のログから読み直したもの）。通知は出さない
                HistEvent e{clean(m.strings[0], 19), clean(m.strings[1], 24), clean(m.strings[2], 120), clean(m.strings[3], 120),
                            clean(m.strings[4], 200)};
                if (history.add(e)) hist_added++;
            } else if (m.address == "/wrist/notify") {
                received++;
                Toast t;
                if (m.strings.size() >= 3) t = {m.strings[0], m.strings[1], m.strings[2]};
                else if (m.strings.size() == 2) t = {"info", m.strings[0], m.strings[1]};
                else t = {"info", "VRChat", m.strings[0]};
                t.kind = clean(t.kind, 24);
                t.title = clean(t.title, 120);
                t.text = clean(t.text, 240);
                // 4〜6 個目: 時刻、ワールド、重複を除く鍵（新しい Windows 側が付ける）。7 個目が "quiet" なら履歴だけ
                std::string ts = m.strings.size() >= 4 ? clean(m.strings[3], 19) : "";
                std::string world = m.strings.size() >= 5 ? clean(m.strings[4], 120) : "";
                std::string key = m.strings.size() >= 6 ? clean(m.strings[5], 200) : "";
                bool quiet = m.strings.size() >= 7 && m.strings[6] == "quiet";
                if (t.kind == "world") current_world = t.title;
                else if (world.empty()) world = current_world;
                if (t.kind == "world") world = t.title;
                // 履歴に残すのは、時刻の付いた VRChat の出来事だけ（-Test や古い Windows 側の通知は残さない）
                if (!ts.empty() && history.add({ts, t.kind, t.title, world, key})) hist_added++;
                if (quiet) continue;
                if (now - window_start > 1.0) { window_start = now; window_count = 0; }
                if (++window_count > 10 || int(queue.size()) >= c.max_queue) {
                    dropped++;
                    logf("多すぎるので捨てました: [%s] %s", t.kind.c_str(), t.title.c_str());
                } else {
                    logf("受信 %s: [%s] %s / %s", ipstr, t.kind.c_str(), t.title.c_str(), t.text.c_str());
                    queue.push_back(t);
                }
            } else {
                logf("%s から知らないアドレス %s を受け取ったので捨てました", ipstr, m.address.c_str());
            }
        }

        if (v.up) {
            // 履歴のパネル（開いているときだけ描き直す。閉じているあいだに変わった分は、開いたときに描く）
            if (v.panel != vr::k_ulOverlayHandleInvalid) {
                if (!thumb_done) {
                    thumb_done = upload(v, v.thumb, v.thumb_tex, panel.render_thumbnail(), HistoryPanel::kThumb, HistoryPanel::kThumb);
                }
                if (history.version() != panel_version) panel_dirty = true;
                bool first = v.panel_tex == nullptr && v.use_vulkan;
                if (panel_dirty && (panel_visible || first || !v.use_vulkan)) {
                    if (upload(v, v.panel, v.panel_tex, panel.render(history.events()), HistoryPanel::kW, HistoryPanel::kH)) {
                        panel_dirty = false;
                        panel_version = history.version();
                    }
                }
            }
            if (showing) {
                double age = now - shown_at, fade = 0.4;
                if (age >= shown_dur) {
                    vr::VROverlay()->HideOverlay(v.toast);
                    showing = false;
                } else {
                    double alpha = age < fade ? age / fade : (shown_dur - age < fade ? (shown_dur - age) / fade : 1.0);
                    vr::VROverlay()->SetOverlayAlpha(v.toast, float(alpha));
                }
            }
            if (!showing && !queue.empty()) {
                Toast t = queue.front();
                queue.pop_front();
                cairo_surface_t* s = render_toast_surface(t, c.font);
                std::vector<uint8_t> px = draw::to_rgba(s);
                cairo_surface_destroy(s);
                if (upload(v, v.toast, v.toast_tex, px, kW, kH)) {
                    place(v, c);
                    vr::VROverlay()->SetOverlayAlpha(v.toast, 0.0f);
                    vr::VROverlay()->ShowOverlay(v.toast);
                    showing = true;
                    shown_at = now;
                    shown_dur = c.duration_s * (is_alert(t.kind) ? 1.5 : 1.0);   // 急ぎは長めに出す
                    shown++;
                    logf("表示: [%s] %s", t.kind.c_str(), t.title.c_str());
                }
            }
        } else if (!queue.empty()) {
            logf("SteamVR が動いていないので表示しません: %zu 件", queue.size());
            dropped += long(queue.size());
            queue.clear();
        }
        if (hist_added / 200 != hist_logged / 200) {
            hist_logged = hist_added;
            logf("履歴に追加: これまで %ld 件", hist_added);
        }
    }
    vr_stop(v);
    close(sock);
    logf("終了: 通知 受信 %ld / 表示 %ld / 捨てた %ld、履歴に追加 %ld 件（全 %zu 件）", received, shown, dropped, hist_added,
         history.events().size());
    return 0;
}
