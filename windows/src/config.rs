// SPDX-License-Identifier: MIT
//! %APPDATA%\vrc-wrist-notifier\config.json（0.4 までの PowerShell 版と同じ項目・同じ既定値）
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    pub frame_host: String,
    pub frame_port: u16,
    pub log_dir: PathBuf,
    pub batch_seconds: f64,
    pub quiet_seconds: f64,
    pub notify_join: bool,
    pub notify_leave: bool,
    pub notify_world: bool,
    pub notify_invite: bool,
    pub notify_friend: bool,
    pub notify_other: bool,
    pub backfill_history: bool,
    pub friends: bool,
    pub friends_refresh_minutes: f64,
    pub friends_file: PathBuf,
    pub wsl_distro: String,
}

pub fn env_dir(var: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(var).unwrap_or_default())
}

pub fn default_path() -> PathBuf {
    env_dir("APPDATA").join("vrc-wrist-notifier").join("config.json")
}

/// アプリのファイル（exe、friends.json、ログ）の置き場所
pub fn app_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("vrc-wrist-notifier")
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        _ => true,
    }
}
fn num(v: &Value, d: f64) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(d),
        Value::String(s) => s.trim().parse().unwrap_or(d),
        _ => d,
    }
}
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        x => x.to_string(),
    }
}

impl Config {
    /// 読めない項目は既定値のまま。知らない項目は warnings に（"_" で始まるものはコメント扱い）
    pub fn load(path: &Path) -> (Config, Vec<String>) {
        let mut c = Config {
            frame_host: "frame".into(),
            frame_port: 39570,
            log_dir: PathBuf::new(),
            batch_seconds: 3.0,
            quiet_seconds: 10.0,
            notify_join: true,
            notify_leave: true,
            notify_world: true,
            notify_invite: true,
            notify_friend: true,
            notify_other: false,
            backfill_history: true,
            friends: true,
            friends_refresh_minutes: 60.0,
            friends_file: PathBuf::new(),
            wsl_distro: "Ubuntu".into(),
        };
        let mut warn = Vec::new();
        if let Ok(raw) = std::fs::read_to_string(path) {
            let raw = raw.trim_start_matches('\u{feff}');
            match serde_json::from_str::<Value>(raw) {
                Ok(Value::Object(m)) => {
                    for (k, v) in m {
                        match k.as_str() {
                            "frame_host" => c.frame_host = text(&v),
                            "frame_port" => c.frame_port = num(&v, 39570.0) as u16,
                            "log_dir" => c.log_dir = PathBuf::from(text(&v)),
                            "batch_seconds" => c.batch_seconds = num(&v, 3.0),
                            "quiet_seconds" => c.quiet_seconds = num(&v, 10.0),
                            "notify_join" => c.notify_join = truthy(&v),
                            "notify_leave" => c.notify_leave = truthy(&v),
                            "notify_world" => c.notify_world = truthy(&v),
                            "notify_invite" => c.notify_invite = truthy(&v),
                            "notify_friend" => c.notify_friend = truthy(&v),
                            "notify_other" => c.notify_other = truthy(&v),
                            "backfill_history" => c.backfill_history = truthy(&v),
                            "friends" => c.friends = truthy(&v),
                            "friends_refresh_minutes" => c.friends_refresh_minutes = num(&v, 60.0),
                            "friends_file" => c.friends_file = PathBuf::from(text(&v)),
                            "wsl_distro" => c.wsl_distro = text(&v),
                            k if k.starts_with('_') => {}
                            k => warn.push(format!("知らない設定項目 {k} は無視します")),
                        }
                    }
                }
                Ok(_) => warn.push(format!("{} がオブジェクトではありません", path.display())),
                Err(e) => warn.push(format!("{} を読めません: {e}", path.display())),
            }
        }
        if c.log_dir.as_os_str().is_empty() {
            let mut low = std::env::var_os("LOCALAPPDATA").unwrap_or_default();
            low.push("Low");
            c.log_dir = PathBuf::from(low).join("VRChat").join("VRChat");
        }
        if c.friends_file.as_os_str().is_empty() {
            c.friends_file = app_dir().join("friends.json");
        }
        (c, warn)
    }
}
