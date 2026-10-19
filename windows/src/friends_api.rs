// SPDX-License-Identifier: MIT
//! キーを読む（Windows の環境変数 → WSL の zsh）→ GET /api/1/auth/user → friends の ID だけ返す。
//! キーはここで使うだけで、返さない・書かない・表示しない。
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vwn::watcher::FetchResult;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MARK: &str = "__VWN_KEY__";

fn key_from_windows() -> Option<String> {
    if let Ok(k) = std::env::var("VRCHAT_AUTH_KEY") {
        if !k.is_empty() {
            return Some(k);
        }
    }
    // 起動したあとにユーザーの環境変数を足したときのため、レジストリ（HKCU\Environment）も見る
    let env = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey("Environment").ok()?;
    env.get_value::<String, _>("VRCHAT_AUTH_KEY").ok().filter(|k| !k.is_empty())
}

/// 対話ログインシェルの zsh で読む（~/.zshrc で設定している場合）。出力は印の付いた行だけ使う
fn key_from_wsl(distro: &str) -> Result<Option<String>, String> {
    let mut child = match Command::new("wsl.exe")
        .args(["-d", distro, "--exec", "zsh", "-lic", &format!("print -r -- {MARK}$VRCHAT_AUTH_KEY")])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return Ok(None), // WSL が無い
    };
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = stdout.read_to_end(&mut s);
        s
    });
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if t0.elapsed() > Duration::from_secs(90) => {
                let _ = child.kill();
                return Err("wsl-timeout".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    Ok(text.lines().filter_map(|l| l.find(MARK).map(|i| l[i + MARK.len()..].trim().to_string())).last().filter(|k| !k.is_empty()))
}

pub fn fetch(distro: String, ua: String) -> FetchResult {
    let mut r = FetchResult::default();
    let mut key = key_from_windows();
    if key.is_some() {
        r.where_ = "windows".into();
    } else if !distro.is_empty() {
        match key_from_wsl(&distro) {
            Ok(k) => key = k,
            Err(code) => {
                r.code = code;
                return r;
            }
        }
        if key.is_some() {
            r.where_ = "wsl".into();
        }
    }
    let Some(key) = key else {
        r.code = "nokey".into();
        return r;
    };
    let tls = match native_tls::TlsConnector::new() {
        Ok(t) => t,
        Err(_) => {
            r.code = "error".into();
            return r;
        }
    };
    let agent = ureq::AgentBuilder::new().tls_connector(Arc::new(tls)).timeout(Duration::from_secs(30)).build();
    let resp = agent
        .get("https://api.vrchat.cloud/api/1/auth/user")
        .set("User-Agent", &ua)
        .set("Accept", "application/json")
        .set("Cookie", &format!("auth={key}"))
        .call();
    drop(key);
    match resp {
        Ok(resp) => {
            let Ok(u) = resp.into_string().map_err(|_| ()).and_then(|b| serde_json::from_str::<serde_json::Value>(&b).map_err(|_| ())) else {
                r.code = "error".into();
                return r;
            };
            let Some(friends) = u["friends"].as_array() else {
                r.code = "nofriends".into(); // 2 段階認証の途中など
                return r;
            };
            r.ids = friends.iter().filter_map(|x| x.as_str()).filter(|s| s.starts_with("usr_")).map(String::from).collect();
            if let Some(id) = u["id"].as_str().filter(|s| s.starts_with("usr_")) {
                r.self_id = id.to_string();
            }
            r.ok = true;
            r.code = "200".into();
        }
        Err(ureq::Error::Status(code, _)) => r.code = code.to_string(),
        Err(_) => r.code = "net".into(),
    }
    r
}
