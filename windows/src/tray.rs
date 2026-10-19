// SPDX-License-Identifier: MIT
//! システムトレイ。見張りは裏のスレッドで 0.5 秒ごとに回し、トレイは状態が変わったときだけ書き換える。
//! 表示は「問題があるときだけ言う」: 何も無ければツールチップは名前だけ、アイコンも普通のまま。
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use vwn::logf;
use vwn::watcher::{Status, Watcher};
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT, WM_QUIT,
};

enum Cmd {
    Refresh,
}

fn icon(warn: bool) -> Icon {
    let rgba: &[u8] = if warn { include_bytes!("../assets/icon-warn-32.rgba") } else { include_bytes!("../assets/icon-32.rgba") };
    Icon::from_rgba(rgba.to_vec(), 32, 32).expect("icon")
}

/// 問題だけを並べる（無ければ空）
pub fn problems(s: &Status, host: &str) -> Vec<String> {
    let mut v = Vec::new();
    if s.send_error.is_some() {
        v.push(format!("⚠ Frame（{host}）に送れません"));
    }
    if s.log == "errors" {
        v.push("⚠ VRChat のログ出力が Full ではありません".into());
    }
    match s.friends.as_str() {
        "nokey" => v.push("⚠ VRCHAT_AUTH_KEY 未設定".into()),
        "bad" => v.push("⚠ VRCHAT_AUTH_KEY が使えません".into()),
        _ => {}
    }
    v
}

struct Ui {
    tray: TrayIcon,
    refresh: MenuId,
    open_log: MenuId,
    quit: MenuId,
}

fn build_menu(s: &Status, host: &str) -> (Menu, MenuId, MenuId, MenuId) {
    let menu = Menu::new();
    let _ = menu.append(&MenuItem::new(format!("vrc-wrist-notifier {}", vwn::VERSION), false, None));
    for p in problems(s, host) {
        let _ = menu.append(&MenuItem::new(p, false, None));
    }
    if s.refreshing {
        let _ = menu.append(&MenuItem::new("フレンド一覧を取り直しています…", false, None));
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    let refresh = MenuItem::new("フレンド一覧を取り直す", !s.refreshing, None);
    let open_log = MenuItem::new("ログを開く", vwn::log::path().is_some(), None);
    let quit = MenuItem::new("終了", true, None);
    let _ = menu.append(&refresh);
    let _ = menu.append(&open_log);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&quit);
    (menu, refresh.id().clone(), open_log.id().clone(), quit.id().clone())
}

fn tooltip(s: &Status, host: &str) -> String {
    let mut t = vec!["vrc-wrist-notifier".to_string()];
    t.extend(problems(s, host));
    let mut s = t.join("\n");
    while s.encode_utf16().count() > 127 {
        s.pop();
    }
    s
}

impl Ui {
    fn new(s: &Status, host: &str) -> Ui {
        let (menu, refresh, open_log, quit) = build_menu(s, host);
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(tooltip(s, host))
            .with_icon(icon(!problems(s, host).is_empty()))
            .build()
            .expect("トレイのアイコンを作れませんでした");
        Ui { tray, refresh, open_log, quit }
    }
    fn update(&mut self, s: &Status, host: &str) {
        let (menu, refresh, open_log, quit) = build_menu(s, host);
        self.tray.set_menu(Some(Box::new(menu)));
        let _ = self.tray.set_tooltip(Some(tooltip(s, host)));
        let _ = self.tray.set_icon(Some(icon(!problems(s, host).is_empty())));
        (self.refresh, self.open_log, self.quit) = (refresh, open_log, quit);
    }
}

fn open_path(p: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let w: Vec<u16> = p.as_os_str().encode_wide().chain([0]).collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), w.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

pub fn run(mut w: Watcher) {
    // 1 つだけ動かす（サインインのたびの自動起動と、手で起動したのとが重ならないように）
    let name: Vec<u16> = "Local\\vrc-wrist-notifier\0".encode_utf16().collect();
    let _mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        logf!("もう動いているので、こちらは終わります");
        return;
    }
    let host = w.cfg.frame_host.clone();
    let shared = Arc::new(Mutex::new(w.status()));
    let (tx, rx) = mpsc::channel::<Cmd>();
    {
        let shared = shared.clone();
        std::thread::spawn(move || {
            w.start();
            loop {
                while let Ok(c) = rx.try_recv() {
                    match c {
                        Cmd::Refresh => w.manual_refresh(),
                    }
                }
                w.tick();
                *shared.lock().unwrap() = w.status();
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }

    let mut shown = shared.lock().unwrap().clone();
    let mut ui = Ui::new(&shown, &host);
    let menu_rx = MenuEvent::receiver();
    loop {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    return;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == ui.quit {
                logf!("トレイの「終了」で終わります");
                drop(ui); // アイコンを消してから
                std::process::exit(0);
            } else if ev.id == ui.refresh {
                let _ = tx.send(Cmd::Refresh);
            } else if ev.id == ui.open_log {
                if let Some(p) = vwn::log::path() {
                    open_path(&p);
                }
            }
        }
        let now = shared.lock().unwrap().clone();
        // 表示に関わるところが変わったときだけ書き換える（ワールド名などでは書き換えない）
        let key = |s: &Status| (problems(s, &host), s.refreshing);
        if key(&now) != key(&shown) {
            ui.update(&now, &host);
        }
        shown = now;
        unsafe {
            MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 200, QS_ALLINPUT);
        }
    }
}
