// SPDX-License-Identifier: MIT
//! SteamVR のオーバーレイ（Windows だけ）。通知のカードを目の前に出し、ダッシュボードに「入退室の履歴」を置く。
//! SteamVR は自分からは起動しない: vrserver.exe が動いているときだけつなぎ、SteamVR が終わったら離れて、また動くのを待つ。
//! openvr_api.dll は SteamVR に入っているものを使う（exe 1 つで配れるように、いっしょには配らない）。
use std::ffi::{c_char, c_void, CString};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::d3d::{Gpu, Surface};
use vwn::upload::{should_upload, Next, TexturePath};
use vwn::config::Config;
use vwn::hub::Hub;
use vwn::logf;
use vwn::panel;
use vwn::toast::{self, Toast};
use vwn::watcher::{Out, Status};

const TOAST_KEY: &str = "cympfh.vrc-wrist-notifier";
const PANEL_KEY: &str = "cympfh.vrc-wrist-notifier.history";
const FADE: f64 = 0.4;

type H = u64;
const INVALID: H = 0;

#[repr(C)]
#[derive(Clone, Copy)]
struct Event {
    kind: u32,
    device: u32,
    age: f32,
    data: [u64; 6], // 8 バイト揃えなので 16 バイト目から
}
#[repr(C)]
struct Matrix34([[f32; 4]; 3]);
#[repr(C)]
struct Vec2(f32, f32);
#[repr(C)]
struct Texture {
    handle: *mut c_void,
    kind: i32,
    color_space: i32,
}

const EV_MOUSE_MOVE: u32 = 300;
const EV_MOUSE_DOWN: u32 = 301;
const EV_MOUSE_UP: u32 = 302;
const EV_FOCUS_LEAVE: u32 = 304;
const EV_SCROLL_DISCRETE: u32 = 305;
const EV_SCROLL_SMOOTH: u32 = 309;
const EV_SHOWN: u32 = 500;
const EV_HIDDEN: u32 = 501;
const EV_QUIT: u32 = 700;
const FLAG_SMOOTH_SCROLL: i32 = 1 << 17;
const INPUT_MOUSE: i32 = 1;

// IVROverlay_028 の関数表での位置（openvr_capi.h の VR_IVROverlay_FnTable の並び、全 82 個）
const OV_FIND_OVERLAY: usize = 0;
const OV_CREATE_OVERLAY: usize = 1;
const OV_DESTROY_OVERLAY: usize = 3;
const OV_SET_OVERLAY_FLAG: usize = 11;
const OV_SET_OVERLAY_ALPHA: usize = 16;
const OV_SET_OVERLAY_SORT_ORDER: usize = 20;
const OV_SET_OVERLAY_WIDTH_IN_METERS: usize = 22;
const OV_SET_OVERLAY_TRANSFORM_TRACKED_DEVICE_RELATIVE: usize = 35;
const OV_SHOW_OVERLAY: usize = 43;
const OV_HIDE_OVERLAY: usize = 44;
const OV_POLL_NEXT_OVERLAY_EVENT: usize = 48;
const OV_SET_OVERLAY_INPUT_METHOD: usize = 50;
const OV_SET_OVERLAY_MOUSE_SCALE: usize = 52;
const OV_SET_OVERLAY_TEXTURE: usize = 60;
const OV_SET_OVERLAY_RAW: usize = 62;
const OV_CREATE_DASHBOARD_OVERLAY: usize = 67;
// IVRSystem_026 の関数表での位置（VR_IVRSystem_FnTable、全 51 個）
const SYS_GET_DXGI_OUTPUT_INFO: usize = 8;
const SYS_POLL_NEXT_EVENT: usize = 30;
const TEXTURE_TYPE_DIRECTX: i32 = 0;
const COLOR_SPACE_AUTO: i32 = 0;

/// VR_InitInternal のアプリの種類。Background は SteamVR を起動しない（動いていなければ 121 で失敗する）
const APP_OVERLAY: i32 = 2;
const APP_BACKGROUND: i32 = 3;
const ERR_NO_SERVER_FOR_BACKGROUND: i32 = 121;

/// IVROverlay_028 / IVRSystem_026 の関数表（順番は openvr_capi.h のとおり）
struct Api {
    _lib: libloading::Library,
    ov: *const *const c_void,
    sys: *const *const c_void,
}

macro_rules! call {
    ($tab:expr, $i:expr, fn($($t:ty),*) -> $r:ty, $($a:expr),*) => {{
        let f: unsafe extern "C" fn($($t),*) -> $r = std::mem::transmute(*$tab.add($i));
        f($($a),*)
    }};
}

impl Api {
    /// 戻り値の Err の i32 は VR_InitInternal のエラー（0 ならそれ以外の失敗）
    fn connect(runtime: &std::path::Path, app: i32) -> Result<Api, (i32, String)> {
        let dll = runtime.join("bin").join("win64").join("openvr_api.dll");
        unsafe {
            let lib = libloading::Library::new(&dll).map_err(|e| (0, format!("{} を読めません（{e}）", dll.display())))?;
            let init: libloading::Symbol<unsafe extern "C" fn(*mut i32, i32) -> isize> =
                lib.get(b"VR_InitInternal\0").map_err(|e| (0, e.to_string()))?;
            let get: libloading::Symbol<unsafe extern "C" fn(*const c_char, *mut i32) -> isize> =
                lib.get(b"VR_GetGenericInterface\0").map_err(|e| (0, e.to_string()))?;
            let mut err = 0;
            init(&mut err, app);
            if err != 0 {
                return Err((err, format!("VR_InitInternal: {err}")));
            }
            let table = |name: &str| {
                let c = CString::new(format!("FnTable:{name}")).unwrap();
                let mut e = 0;
                let p = get(c.as_ptr(), &mut e);
                if e != 0 || p == 0 { Err(format!("{name}: {e}")) } else { Ok(p as *const *const c_void) }
            };
            let ov = table("IVROverlay_028");
            let sys = table("IVRSystem_026");
            match (ov, sys) {
                (Ok(ov), Ok(sys)) => Ok(Api { _lib: lib, ov, sys }),
                (Err(e), _) | (_, Err(e)) => {
                    if let Ok(shut) = lib.get::<unsafe extern "C" fn()>(b"VR_ShutdownInternal\0") {
                        shut();
                    }
                    Err((0, e))
                }
            }
        }
    }
    fn find(&self, key: &str) -> H {
        let k = CString::new(key).unwrap();
        let mut h = INVALID;
        unsafe { call!(self.ov, OV_FIND_OVERLAY, fn(*const c_char, *mut H) -> i32, k.as_ptr(), &mut h) };
        h
    }
    fn create(&self, key: &str, name: &str) -> Result<H, i32> {
        let (k, n) = (CString::new(key).unwrap(), CString::new(name).unwrap());
        let mut h = INVALID;
        let e = unsafe { call!(self.ov, OV_CREATE_OVERLAY, fn(*const c_char, *const c_char, *mut H) -> i32, k.as_ptr(), n.as_ptr(), &mut h) };
        if e == 0 { Ok(h) } else { Err(e) }
    }
    fn create_dashboard(&self, key: &str, name: &str) -> Result<(H, H), i32> {
        let (k, n) = (CString::new(key).unwrap(), CString::new(name).unwrap());
        let (mut m, mut t) = (INVALID, INVALID);
        let e = unsafe {
            call!(self.ov, OV_CREATE_DASHBOARD_OVERLAY, fn(*const c_char, *const c_char, *mut H, *mut H) -> i32, k.as_ptr(), n.as_ptr(), &mut m, &mut t)
        };
        if e == 0 { Ok((m, t)) } else { Err(e) }
    }
    fn destroy(&self, h: H) {
        unsafe { call!(self.ov, OV_DESTROY_OVERLAY, fn(H) -> i32, h) };
    }
    fn flag(&self, h: H, f: i32, on: bool) {
        unsafe { call!(self.ov, OV_SET_OVERLAY_FLAG, fn(H, i32, bool) -> i32, h, f, on) };
    }
    fn alpha(&self, h: H, a: f32) {
        unsafe { call!(self.ov, OV_SET_OVERLAY_ALPHA, fn(H, f32) -> i32, h, a) };
    }
    fn sort_order(&self, h: H, o: u32) {
        unsafe { call!(self.ov, OV_SET_OVERLAY_SORT_ORDER, fn(H, u32) -> i32, h, o) };
    }
    fn width(&self, h: H, m: f32) {
        unsafe { call!(self.ov, OV_SET_OVERLAY_WIDTH_IN_METERS, fn(H, f32) -> i32, h, m) };
    }
    fn place_on_head(&self, h: H, y: f32, dist: f32) {
        let m = Matrix34([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, -dist]]);
        unsafe { call!(self.ov, OV_SET_OVERLAY_TRANSFORM_TRACKED_DEVICE_RELATIVE, fn(H, u32, *const Matrix34) -> i32, h, 0, &m) };
    }
    fn show(&self, h: H) {
        unsafe { call!(self.ov, OV_SHOW_OVERLAY, fn(H) -> i32, h) };
    }
    fn hide(&self, h: H) {
        unsafe { call!(self.ov, OV_HIDE_OVERLAY, fn(H) -> i32, h) };
    }
    fn poll_overlay(&self, h: H) -> Option<Event> {
        let mut e: Event = unsafe { std::mem::zeroed() };
        let ok = unsafe { call!(self.ov, OV_POLL_NEXT_OVERLAY_EVENT, fn(H, *mut Event, u32) -> bool, h, &mut e, std::mem::size_of::<Event>() as u32) };
        ok.then_some(e)
    }
    fn input_mouse(&self, h: H, w: f32, hgt: f32) {
        unsafe {
            call!(self.ov, OV_SET_OVERLAY_INPUT_METHOD, fn(H, i32) -> i32, h, INPUT_MOUSE);
            call!(self.ov, OV_SET_OVERLAY_MOUSE_SCALE, fn(H, *const Vec2) -> i32, h, &Vec2(w, hgt));
        }
    }
    fn raw(&self, h: H, rgba: &mut [u8], w: u32, hgt: u32) -> i32 {
        unsafe { call!(self.ov, OV_SET_OVERLAY_RAW, fn(H, *mut c_void, u32, u32, u32) -> i32, h, rgba.as_mut_ptr() as *mut c_void, w, hgt, 4) }
    }
    fn texture(&self, h: H, tex: *mut c_void) -> i32 {
        let t = Texture { handle: tex, kind: TEXTURE_TYPE_DIRECTX, color_space: COLOR_SPACE_AUTO };
        unsafe { call!(self.ov, OV_SET_OVERLAY_TEXTURE, fn(H, *const Texture) -> i32, h, &t) }
    }
    /// SteamVR が使っている GPU の番号（DXGI のアダプタ）
    fn dxgi_adapter(&self) -> Option<u32> {
        let mut i: i32 = -1;
        unsafe { call!(self.sys, SYS_GET_DXGI_OUTPUT_INFO, fn(*mut i32) -> (), &mut i) };
        (i >= 0).then_some(i as u32)
    }
    fn poll_system(&self) -> Option<Event> {
        let mut e: Event = unsafe { std::mem::zeroed() };
        let ok = unsafe { call!(self.sys, SYS_POLL_NEXT_EVENT, fn(*mut Event, u32) -> bool, &mut e, std::mem::size_of::<Event>() as u32) };
        ok.then_some(e)
    }
}

impl Drop for Api {
    fn drop(&mut self) {
        unsafe {
            if let Ok(shut) = self._lib.get::<unsafe extern "C" fn()>(b"VR_ShutdownInternal\0") {
                shut();
            }
        }
    }
}

fn f32_at(e: &Event, i: usize) -> f32 {
    let words: [u32; 12] = unsafe { std::mem::transmute(e.data) };
    f32::from_bits(words[i])
}

// ---------------------------------------------------------------- SteamVR が動いているか
/// 動いている vrserver.exe の PID（無ければ None）。動いていないときに OpenVR を初期化すると SteamVR が起動してしまうので、先に確かめる
pub fn vrserver_pid() -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut ok = Process32FirstW(snap, &mut pe) != 0;
        while ok {
            let n = pe.szExeFile.iter().position(|&c| c == 0).unwrap_or(pe.szExeFile.len());
            if String::from_utf16_lossy(&pe.szExeFile[..n]).eq_ignore_ascii_case("vrserver.exe") {
                found = Some(pe.th32ProcessID);
                break;
            }
            ok = Process32NextW(snap, &mut pe) != 0;
        }
        CloseHandle(snap);
        found
    }
}

/// SteamVR の入っている場所（%LOCALAPPDATA%\openvr\openvrpaths.vrpath の runtime）
fn runtime_dir() -> Option<PathBuf> {
    let p = vwn::config::env_dir("LOCALAPPDATA").join("openvr").join("openvrpaths.vrpath");
    let raw = std::fs::read_to_string(p).ok()?;
    let j: serde_json::Value = serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    j["runtime"].as_array()?.iter().filter_map(|x| x.as_str()).map(PathBuf::from).find(|d| d.join("bin/win64/openvr_api.dll").exists())
}

// ---------------------------------------------------------------- つながっている間
struct Session {
    api: Api,
    /// つないだときの vrserver.exe の PID
    pid: u32,
    /// vrserver.exe がまだ動いているかを最後に確かめた時刻
    alive_at: Instant,
    toast: H,
    panel: H,
    thumb: H,
    panel_visible: bool,
    panel_version: Option<u64>,
    panel_dirty: bool,
    checked_at: Instant,
    showing: Option<(Instant, f64)>,
    /// 絵は作りっぱなしの D3D11 テクスチャで渡す（d3d.rs）。作れなければ SetOverlayRaw（ちらつく）
    gpu: Option<Gpu>,
    toast_sf: Option<Surface>,
    panel_sf: Option<Surface>,
    tex_path: TexturePath,
    /// 最後に渡した中身（同じなら渡さない）
    toast_last: Vec<u8>,
    panel_last: Vec<u8>,
}

impl Session {
    /// オーバーレイを作る。strict（Background でつないだとき）なら、通知かパネルが作れなければ None にして、
    /// 呼んだ側が Overlay でつなぎ直す
    fn open(api: Api, cfg: &Config, pid: u32, strict: bool) -> Option<Session> {
        let toast = match api.find(TOAST_KEY) {
            INVALID => api.create(TOAST_KEY, "VRChat の通知").unwrap_or_else(|e| {
                if !strict {
                    logf!("通知のオーバーレイを作れません（{e}）");
                }
                INVALID
            }),
            h => h,
        };
        let mut s = Session {
            api, pid, toast, panel: INVALID, thumb: INVALID, panel_visible: false, panel_version: None, panel_dirty: true,
            checked_at: Instant::now(), alive_at: Instant::now(), showing: None, gpu: None, toast_sf: None, panel_sf: None,
            tex_path: TexturePath::default(), toast_last: Vec::new(), panel_last: Vec::new(),
        };
        if cfg.history_panel {
            s.make_panel(cfg, strict);
        }
        if strict && (toast == INVALID || (cfg.history_panel && s.panel == INVALID)) {
            return None; // Drop で作れた分は消してから離れる
        }
        match Gpu::new(s.api.dxgi_adapter()) {
            Ok(g) => s.gpu = Some(g),
            Err(e) => logf!("D3D11 を使えないので SetOverlayRaw で渡します（ちらつくことがあります）: {e}"),
        }
        if toast != INVALID {
            s.api.width(toast, cfg.toast_width_m as f32);
            s.api.sort_order(toast, 200);
            s.api.hide(toast);
            logf!("SteamVR につながりました。通知のオーバーレイ {TOAST_KEY} を登録しました");
        }
        if s.panel != INVALID {
            logf!("ダッシュボードに「入退室の履歴」（{PANEL_KEY}）を登録しました");
        }
        Some(s)
    }

    fn make_panel(&mut self, cfg: &Config, quiet: bool) {
        match self.api.create_dashboard(PANEL_KEY, "入退室の履歴") {
            Ok((m, t)) => {
                self.panel = m;
                self.thumb = t;
                self.api.width(m, cfg.panel_width_m as f32);
                self.api.input_mouse(m, panel::W as f32, panel::H as f32);
                self.api.flag(m, FLAG_SMOOTH_SCROLL, true);
                let mut icon = icon_rgba();
                self.api.raw(t, &mut icon.0, icon.1, icon.2);
                self.panel_dirty = true;
                self.panel_version = None;
                self.panel_last.clear(); // 作り直したパネルには、同じ絵でも渡しなおす
            }
            Err(e) if !quiet => logf!("ダッシュボードのパネルを作れません（{e}）"),
            Err(_) => {}
        }
    }

    /// 戻り値 false = SteamVR が終わる
    fn frame(&mut self, hub: &mut Hub, cfg: &Config) -> bool {
        while let Some(e) = self.api.poll_system() {
            if e.kind == EV_QUIT {
                logf!("SteamVR が終わるので離れます");
                return false;
            }
        }
        // パネルが消されていたら作り直す（5 秒ごとに確かめる）
        if cfg.history_panel && self.checked_at.elapsed() >= Duration::from_secs(5) {
            self.checked_at = Instant::now();
            if self.api.find(PANEL_KEY) == INVALID {
                self.make_panel(cfg, false);
                if self.panel != INVALID {
                    logf!("ダッシュボードの「入退室の履歴」を作り直しました");
                }
            }
        }
        if self.panel != INVALID {
            while let Some(e) = self.api.poll_overlay(self.panel) {
                let (x, y) = (f32_at(&e, 0), panel::H as f32 - f32_at(&e, 1));
                let p = &mut hub.panel;
                let changed = match e.kind {
                    EV_MOUSE_MOVE => p.pointer_move(x, y),
                    EV_MOUSE_DOWN => p.pointer_down(x, y),
                    EV_MOUSE_UP => p.pointer_up(x, y),
                    EV_FOCUS_LEAVE => p.pointer_leave(),
                    EV_SCROLL_DISCRETE | EV_SCROLL_SMOOTH => p.scroll(f32_at(&e, 1)),
                    EV_SHOWN => {
                        self.panel_visible = true;
                        false
                    }
                    EV_HIDDEN => {
                        self.panel_visible = false;
                        p.pointer_leave()
                    }
                    _ => false,
                };
                self.panel_dirty |= changed;
            }
            hub.save_notify_if_changed();
            if self.panel_version != Some(hub.history.version()) {
                self.panel_dirty = true;
            }
            // 開いているときだけ描き直す（最初の 1 回は、開いたときにすぐ見えるように描いておく）
            if self.panel_dirty && (self.panel_visible || self.panel_version.is_none()) {
                let c = hub.panel.render(hub.history.events());
                let mut px = c.to_rgba();
                if self.upload(true, &mut px, panel::W, panel::H) == 0 {
                    self.panel_dirty = false;
                    self.panel_version = Some(hub.history.version());
                }
            }
        }
        if self.toast == INVALID {
            hub.queue.clear();
            return true;
        }
        if let Some((at, dur)) = self.showing {
            let age = at.elapsed().as_secs_f64();
            if age >= dur {
                self.api.hide(self.toast);
                self.showing = None;
            } else {
                let a = if age < FADE { age / FADE } else if dur - age < FADE { (dur - age) / FADE } else { 1.0 };
                self.api.alpha(self.toast, a as f32);
            }
        }
        if self.showing.is_none() {
            if let Some(t) = hub.queue.pop_front() {
                self.show_toast(&t, cfg);
            }
        }
        true
    }

    /// 絵を渡す（panel = true ならパネル、false なら通知）。前と同じ中身なら何もしないで 0。
    /// D3D11 が使えるときは作りっぱなしのテクスチャを書き換えて SetOverlayTexture → Flush（ちらつかない）。
    /// 失敗したらその回は SetOverlayRaw。続けて失敗する・デバイスが失われたら、D3D11 をやめてずっと SetOverlayRaw
    fn upload(&mut self, panel: bool, rgba: &mut [u8], w: u32, h: u32) -> i32 {
        let last = if panel { &self.panel_last } else { &self.toast_last };
        if !should_upload(last, rgba) {
            return 0;
        }
        let ov = if panel { self.panel } else { self.toast };
        let mut done = None;
        let mut give_up = false;
        if let Some(gpu) = &self.gpu {
            let slot = if panel { &mut self.panel_sf } else { &mut self.toast_sf };
            if slot.is_none() {
                *slot = gpu.surface(w, h).map_err(|e| logf!("テクスチャを作れません: {e}")).ok();
            }
            let e = match slot.as_ref().map(|sf| sf.write(gpu, rgba, w, h)) {
                Some(Ok(tex)) => {
                    let e = self.api.texture(ov, tex);
                    gpu.flush(); // SteamVR はこの後でコピーするので、渡してから流す
                    e
                }
                Some(Err(e)) => {
                    logf!("{e}");
                    -1
                }
                None => -1,
            };
            match self.tex_path.record(e == 0, gpu.removed()) {
                Next::Ok => done = Some(0),
                Next::RawThisTime => {}
                Next::GiveUp => give_up = true,
            }
        }
        if give_up {
            logf!("SetOverlayRaw に切り替えます（ちらつきます）");
            self.drop_gpu();
        }
        let e = match done {
            Some(e) => e,
            None => self.api.raw(ov, rgba, w, h),
        };
        if e == 0 {
            let last = if panel { &mut self.panel_last } else { &mut self.toast_last };
            last.clear();
            last.extend_from_slice(rgba);
        }
        e
    }

    /// テクスチャとデバイスを手放す（SteamVR がつながっていなくても安全に消せる）
    fn drop_gpu(&mut self) {
        self.toast_sf = None;
        self.panel_sf = None;
        self.gpu = None;
    }

    fn show_toast(&mut self, t: &Toast, cfg: &Config) {
        let mut px = toast::render(t).to_rgba();
        let e = self.upload(false, &mut px, toast::W, toast::H);
        if e != 0 {
            logf!("通知を出せません（{e}）");
            return;
        }
        self.api.place_on_head(self.toast, cfg.toast_y_m as f32, cfg.toast_distance_m as f32);
        self.api.width(self.toast, cfg.toast_width_m as f32);
        self.api.alpha(self.toast, 0.0);
        self.api.show(self.toast);
        let dur = cfg.toast_seconds * if toast::is_alert(&t.kind) { 1.5 } else { 1.0 };
        self.showing = Some((Instant::now(), dur));
        logf!("表示: [{}] {}", t.kind, t.title);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for h in [self.toast, self.panel] {
            if h != INVALID {
                self.api.destroy(h);
            }
        }
    }
}

fn icon_rgba() -> (Vec<u8>, u32, u32) {
    let pm = tiny_skia::Pixmap::decode_png(include_bytes!("../assets/icon.png")).expect("icon.png");
    let (w, h) = (pm.width(), pm.height());
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for p in pm.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    (out, w, h)
}

// ---------------------------------------------------------------- スレッド
/// vrserver.exe の pid につなぐ。まず Background（SteamVR を起動しない種類）で、通知とパネルが作れなければ Overlay で。
/// 121（vrserver が今ちょうど終わった）は黙って次の確認を待つ
fn connect(cfg: &Config, pid: u32) -> Result<Session, String> {
    let dir = runtime_dir().ok_or_else(|| "SteamVR の場所がわかりません（openvrpaths.vrpath）".to_string())?;
    match Api::connect(&dir, APP_BACKGROUND) {
        Ok(api) => {
            if let Some(s) = Session::open(api, cfg, pid, true) {
                return Ok(s);
            }
            // Background では作れなかった（Drop で切ってある）。vrserver が動いているのは確かめてあるので、Overlay でも起動はしない
            logf!("Background ではオーバーレイを作れないので、Overlay でつなぎ直します");
        }
        Err((ERR_NO_SERVER_FOR_BACKGROUND, _)) => return Err(String::new()),
        Err((_, e)) => return Err(e),
    }
    if vrserver_pid() != Some(pid) {
        return Err(String::new());
    }
    let api = Api::connect(&dir, APP_OVERLAY).map_err(|(_, e)| e)?;
    Session::open(api, cfg, pid, false).ok_or_else(|| "オーバーレイを作れません".to_string())
}

/// VR のスレッド。見張りから届いたものを履歴と通知にし、SteamVR が動いていれば出す
pub fn run(cfg: Config, cfg_path: PathBuf, rx: Receiver<Out>, status: Arc<Mutex<Status>>) {
    let mut hub = Hub::new(&cfg, cfg_path);
    logf!("履歴: {}（{} 件）", hub.history.path().display(), hub.history.events().len());
    let mut session: Option<Session> = None;
    let mut checked: Option<Instant> = None;
    // SteamVR が終わると言った vrserver.exe の PID。同じ PID が残っているあいだはつなぎ直さない（別の PID になれば、すぐつなぐ）
    let mut quit_pid: Option<u32> = None;
    let mut last_err = String::new();
    let mut dropped = 0usize;
    loop {
        while let Ok(o) = rx.try_recv() {
            hub.handle(o);
        }
        {
            let st = status.lock().unwrap().clone();
            if hub.set_status(&st.log, &st.friends) {
                if let Some(s) = session.as_mut() {
                    s.panel_dirty = true;
                }
            }
        }
        // つながっているあいだも、vrserver.exe が落ちていないか 3 秒ごとに確かめる（VREvent_Quit 無しで落ちたとき）
        if let Some(s) = session.as_mut() {
            if s.alive_at.elapsed() >= Duration::from_secs(3) {
                s.alive_at = Instant::now();
                if vrserver_pid() != Some(s.pid) {
                    logf!("SteamVR が落ちたので離れます");
                    // 死んだ接続には何も呼ばない（止まるかもしれないので、後片付けをせずに手放す）
                    // D3D11 のものは SteamVR と関係なく消せるので、先に取り出して普通に消す（落ちるたびに漏らさない）
                    if let Some(mut dead) = session.take() {
                        dead.drop_gpu();
                        std::mem::forget(dead);
                    }
                    checked = None;
                }
            }
        }
        if session.is_none() && checked.map_or(true, |t| t.elapsed() >= Duration::from_secs(5)) {
            checked = Some(Instant::now());
            match vrserver_pid() {
                None => quit_pid = None,
                Some(pid) if quit_pid == Some(pid) => {}
                Some(pid) => match connect(&cfg, pid) {
                    Ok(s) => {
                        last_err.clear();
                        quit_pid = None;
                        session = Some(s);
                    }
                    Err(e) => {
                        if !e.is_empty() && e != last_err {
                            logf!("SteamVR につなげません: {e}");
                        }
                        last_err = e;
                    }
                },
            }
        }
        let visible = match session.as_mut() {
            Some(s) => {
                if !s.frame(&mut hub, &cfg) {
                    quit_pid = Some(s.pid);
                    session = None;
                    checked = None;
                    false
                } else {
                    s.panel_visible
                }
            }
            None => {
                // SteamVR が動いていないときの通知は出さずに捨てる（履歴には残っている）
                dropped += hub.queue.len();
                hub.queue.clear();
                hub.save_notify_if_changed();
                false
            }
        };
        if dropped > 0 && session.is_none() {
            logf!("SteamVR が動いていないので表示しませんでした: {dropped} 件");
            dropped = 0;
        }
        std::thread::sleep(Duration::from_millis(if visible { 16 } else { 50 }));
    }
}

const _: () = assert!(std::mem::size_of::<Event>() == 64);
