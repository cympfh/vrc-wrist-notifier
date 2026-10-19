// SPDX-License-Identifier: MIT
//! 設定の窓（トレイの「設定…」）。VRChat のキーを伏せ字で入れて「保存」すると、暗号化して保存し（keystore.rs）、
//! すぐにフレンド一覧を取り直す。「消す」で保存したキーを消す。窓はトレイと同じスレッド（メッセージループは tray.rs）。
//! 入れたキーは窓にもログにも出さない。保存したら入力欄は空にする。平文のバッファを 0 で埋めるのはできる範囲で（best effort）。
use std::cell::{Cell, RefCell};
use vwn::auth::{self, Tone};
use vwn::watcher::Status;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetStockObject, SetBkColor, SetTextColor, CLEARTYPE_QUALITY, DEFAULT_CHARSET, HDC, HFONT,
    WHITE_BRUSH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub enum KeyEvent {
    Saved,
    Cleared,
}

const ID_EDIT: usize = 100;
const ID_SAVE: usize = 1; // IDOK: 入力欄で Enter を押したときもこれ
const ID_CLOSE: usize = 2; // IDCANCEL: Esc
const ID_CLEAR: usize = 101;
const ID_HOWTO: usize = 102;
const EM_SETCUEBANNER: u32 = 0x1501;
const HOWTO_URL: &str = "https://github.com/cympfh/vrc-wrist-notifier#vrchat-key";

type Callback = Box<dyn Fn(KeyEvent)>;

#[derive(Clone, Copy)]
struct Hw {
    win: HWND,
    edit: HWND,
    status: HWND,
    msg: HWND,
    clear: HWND,
}

thread_local! {
    static HW: Cell<Option<Hw>> = const { Cell::new(None) };
    static TONE: Cell<Tone> = const { Cell::new(Tone::Neutral) };
    static MSG_TONE: Cell<Tone> = const { Cell::new(Tone::Neutral) };
    static FONTS: RefCell<Vec<HFONT>> = const { RefCell::new(Vec::new()) };
    static CB: RefCell<Option<Callback>> = const { RefCell::new(None) };
    static LAST: RefCell<Option<(String, bool)>> = const { RefCell::new(None) };
    static STATUS: RefCell<Option<Status>> = const { RefCell::new(None) };
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn color(t: Tone) -> u32 {
    // COLORREF は 0x00BBGGRR
    match t {
        Tone::Neutral => 0x00_55_50_4b,
        Tone::Good => 0x00_3c_8a_1e,
        Tone::Warn => 0x00_10_5a_c8,
    }
}

/// 開いている窓（IsDialogMessage で Tab・Enter・Esc を効かせるため）
pub fn hwnd() -> Option<HWND> {
    HW.with(|h| h.get()).map(|h| h.win)
}

fn set_msg(text: &str, tone: Tone) {
    let Some(h) = HW.with(|h| h.get()) else { return };
    MSG_TONE.with(|t| t.set(tone));
    let t = w(text);
    unsafe { SetWindowTextW(h.msg, t.as_ptr()) };
}

/// 状態が変わったら「今の状態」の行と「消す」のボタンを書き換える（窓が開いているときだけ）
pub fn update(s: &Status) {
    let Some(h) = HW.with(|h| h.get()) else { return };
    STATUS.with(|st| *st.borrow_mut() = Some(s.clone()));
    let saved = crate::keystore::exists();
    let (line, tone) = auth::settings_status(saved, &s.friends, s.refreshing, s.friends_count);
    let changed = LAST.with(|l| {
        let mut l = l.borrow_mut();
        let now = Some((line.clone(), saved));
        let c = *l != now;
        *l = now;
        c
    });
    if changed {
        TONE.with(|t| t.set(tone));
        let t = w(&line);
        unsafe {
            SetWindowTextW(h.status, t.as_ptr());
            EnableWindow(h.clear, saved as i32);
        }
    }
}

fn refresh_now() {
    LAST.with(|l| *l.borrow_mut() = None);
    if let Some(s) = STATUS.with(|st| st.borrow().clone()) {
        update(&s);
    }
}

fn fire(ev: KeyEvent) {
    CB.with(|c| {
        if let Some(f) = &*c.borrow() {
            f(ev)
        }
    });
}

fn wipe16(b: &mut [u16]) {
    for x in b.iter_mut() {
        unsafe { std::ptr::write_volatile(x, 0) };
    }
}

fn on_save(h: Hw) {
    let mut buf = vec![0u16; 4096];
    let n = unsafe { GetWindowTextW(h.edit, buf.as_mut_ptr(), buf.len() as i32) }.max(0) as usize;
    let mut text = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
    wipe16(&mut buf);
    let r = auth::normalize_key(&text);
    unsafe { text.as_bytes_mut().fill(0) };
    match r {
        Err(e) => set_msg(e.message(), Tone::Warn),
        Ok(mut key) => {
            let saved = crate::keystore::save(&key);
            unsafe { key.as_bytes_mut().fill(0) };
            match saved {
                Ok(()) => {
                    let empty = w("");
                    unsafe { SetWindowTextW(h.edit, empty.as_ptr()) };
                    set_msg("保存しました（暗号化してこの PC に）", Tone::Good);
                    fire(KeyEvent::Saved);
                }
                Err(e) => set_msg(&format!("保存できませんでした: {e}"), Tone::Warn),
            }
        }
    }
    refresh_now();
}

fn on_clear() {
    match crate::keystore::clear() {
        Ok(()) => {
            set_msg("保存していたキーを消しました", Tone::Neutral);
            fire(KeyEvent::Cleared);
        }
        Err(e) => set_msg(&format!("消せませんでした: {e}"), Tone::Warn),
    }
    refresh_now();
}

fn open_url(url: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let (u, verb) = (w(url), w("open"));
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), u.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let Some(h) = HW.with(|h| h.get()) else { return 0 };
            match wp & 0xffff {
                ID_SAVE => on_save(h),
                ID_CLEAR => on_clear(),
                ID_HOWTO => open_url(HOWTO_URL),
                ID_CLOSE => {
                    DestroyWindow(hwnd);
                }
                _ => return DefWindowProcW(hwnd, msg, wp, lp),
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wp as HDC;
            let ctl = lp as HWND;
            let h = HW.with(|h| h.get());
            let c = match h {
                Some(h) if ctl == h.status => color(TONE.with(|t| t.get())),
                Some(h) if ctl == h.msg => color(MSG_TONE.with(|t| t.get())),
                _ => 0x00_30_2c_28,
            };
            SetTextColor(hdc, c);
            SetBkColor(hdc, 0x00ff_ffff);
            GetStockObject(WHITE_BRUSH) as LRESULT
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DPICHANGED => {
            // 別の DPI のモニターに移った: Windows が勧める大きさにして、文字と子の窓を並べなおす
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, std::ptr::null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            apply_dpi(((wp >> 16) & 0xffff) as u32);
            0
        }
        WM_DESTROY => {
            // トレイのアプリは動き続けるので PostQuitMessage はしない
            HW.with(|h| h.set(None));
            LAST.with(|l| *l.borrow_mut() = None);
            LAYOUT.with(|l| l.borrow_mut().clear());
            FONTS.with(|f| {
                for x in f.borrow_mut().drain(..) {
                    DeleteObject(x as _);
                }
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// 窓を開く（もう開いていれば前に出す）
pub fn open(status: &Status, cb: impl Fn(KeyEvent) + 'static) {
    CB.with(|c| *c.borrow_mut() = Some(Box::new(cb)));
    if let Some(h) = HW.with(|h| h.get()) {
        unsafe {
            ShowWindow(h.win, SW_RESTORE);
            SetForegroundWindow(h.win);
        }
        return;
    }
    unsafe { create() };
    set_msg("", Tone::Neutral);
    update(status);
}

/// 文字の大きさ（96 DPI のときの px）と太さ。LAYOUT の 3 つめはここの番号
const FONT_SPECS: [(f32, bool); 5] = [(26.0, true), (15.0, false), (17.0, false), (14.0, false), (16.0, false)];
const F_HEAD: usize = 0;
const F_BODY: usize = 1;
const F_EDIT: usize = 2;
const F_SMALL: usize = 3;
const F_BTN: usize = 4;
const CLIENT_W: f32 = 620.0;
const CLIENT_H: f32 = 380.0;

thread_local! {
    /// 子の窓と、96 DPI のときの位置・大きさ、文字の番号（DPI が変わったら並べなおすため）
    static LAYOUT: RefCell<Vec<(HWND, [f32; 4], usize)>> = const { RefCell::new(Vec::new()) };
}

fn scale(v: f32, dpi: u32) -> i32 {
    (v * dpi.max(96) as f32 / 96.0).round() as i32
}

/// 文字を作りなおして、子の窓をその DPI で並べなおす
unsafe fn apply_dpi(dpi: u32) {
    let face = w("Yu Gothic UI");
    let fonts: Vec<HFONT> = FONT_SPECS
        .iter()
        .map(|&(px, bold)| {
            CreateFontW(
                -scale(px, dpi), 0, 0, 0, if bold { 600 } else { 400 }, 0, 0, 0, DEFAULT_CHARSET as _, 0, 0,
                CLEARTYPE_QUALITY as _, 0, face.as_ptr(),
            )
        })
        .collect();
    LAYOUT.with(|l| {
        for &(h, [x, y, cw, ch], f) in l.borrow().iter() {
            SetWindowPos(h, std::ptr::null_mut(), scale(x, dpi), scale(y, dpi), scale(cw, dpi), scale(ch, dpi), SWP_NOZORDER | SWP_NOACTIVATE);
            SendMessageW(h, WM_SETFONT, fonts[f] as WPARAM, 1);
        }
    });
    let old = FONTS.with(|v| std::mem::replace(&mut *v.borrow_mut(), fonts));
    for f in old {
        DeleteObject(f as _);
    }
}

unsafe fn create() {
    let hinst = GetModuleHandleW(std::ptr::null());
    let class = w("vrc-wrist-notifier-settings");
    let mut wc: WNDCLASSEXW = std::mem::zeroed();
    wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.hIcon = LoadIconW(hinst, 1 as _); // exe に埋め込んだアイコン（build.rs）
    wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
    wc.hbrBackground = GetStockObject(WHITE_BRUSH) as _;
    wc.lpszClassName = class.as_ptr();
    RegisterClassExW(&wc); // 2 回目以降は失敗するが、登録は残っている

    let dpi = GetDpiForSystem();
    let pad = 36.0;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let mut r = RECT { left: 0, top: 0, right: scale(CLIENT_W, dpi), bottom: scale(CLIENT_H, dpi) };
    AdjustWindowRectEx(&mut r, style, 0, WS_EX_CONTROLPARENT);
    let (ww, wh) = (r.right - r.left, r.bottom - r.top);
    let (sx, sy) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
    let title = w("vrc-wrist-notifier の設定");
    let win = CreateWindowExW(
        WS_EX_CONTROLPARENT, class.as_ptr(), title.as_ptr(), style, (sx - ww) / 2, (sy - wh) / 2, ww, wh,
        std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null(),
    );
    if win.is_null() {
        vwn::logf!("設定の窓を作れませんでした（{}）", std::io::Error::last_os_error());
        return;
    }
    LAYOUT.with(|l| l.borrow_mut().clear());
    let child = |cls: &str, text: &str, st: u32, ex: u32, rect: [f32; 4], id: usize, font: usize| {
        let (c, t) = (w(cls), w(text));
        let h = CreateWindowExW(
            ex, c.as_ptr(), t.as_ptr(), WS_CHILD | WS_VISIBLE | st, 0, 0, 0, 0, win, id as _, hinst, std::ptr::null(),
        );
        LAYOUT.with(|l| l.borrow_mut().push((h, rect, font)));
        h
    };
    let inner = CLIENT_W - 2.0 * pad;
    child("STATIC", "VRChat のキー", 0, 0, [pad, 26.0, inner, 40.0], 0, F_HEAD);
    child(
        "STATIC",
        "フレンドの名前を黄色にするために使います（無くても通知は出ます）。\r\nキーは暗号化して、この PC のこの Windows ユーザーだけが読める形で保存します。",
        0, 0, [pad, 76.0, inner, 48.0], 0, F_SMALL,
    );
    let status = child("STATIC", "", 0, 0, [pad, 134.0, inner, 26.0], 0, F_BODY);
    let edit = child(
        "EDIT", "", (ES_PASSWORD | ES_AUTOHSCROLL) as u32 | WS_TABSTOP, WS_EX_CLIENTEDGE, [pad, 174.0, inner, 42.0], ID_EDIT, F_EDIT,
    );
    let cue = w("authcookie_… をここに貼り付け");
    SendMessageW(edit, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
    let msg = child("STATIC", "", 0, 0, [pad, 224.0, inner, 24.0], 0, F_SMALL);
    let (bh, by) = (46.0, CLIENT_H - pad - 46.0);
    child("BUTTON", "キーの調べ方", BS_PUSHBUTTON as u32 | WS_TABSTOP, 0, [pad, by, 170.0, bh], ID_HOWTO, F_BTN);
    let clear = child("BUTTON", "消す", BS_PUSHBUTTON as u32 | WS_TABSTOP, 0, [CLIENT_W - pad - 260.0, by, 120.0, bh], ID_CLEAR, F_BTN);
    child("BUTTON", "保存", BS_DEFPUSHBUTTON as u32 | WS_TABSTOP, 0, [CLIENT_W - pad - 128.0, by, 128.0, bh], ID_SAVE, F_BTN);
    // 実際に出たモニターの DPI で並べる（システムの DPI と違うモニターのこともある）
    apply_dpi(GetDpiForWindow(win));
    HW.with(|h| h.set(Some(Hw { win, edit, status, msg, clear })));
    ShowWindow(win, SW_SHOWNORMAL);
    SetForegroundWindow(win);
    SetFocus(edit);
}
