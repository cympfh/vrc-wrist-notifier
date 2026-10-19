// SPDX-License-Identifier: MIT
//! %APPDATA%\vrc-wrist-notifier\config.json。無ければ既定値で動く。パネルで選んだ「通知設定」だけはアプリが書き込む
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
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
    /// 通知設定: all / friends / off（パネルで選ぶと書き込まれる）
    pub notify_people: String,
    /// 通知を出す秒数（急ぎの通知は 1.5 倍）
    pub toast_seconds: f64,
    /// 通知のカードの位置: 目の前 toast_distance_m、少し下 toast_y_m、幅 toast_width_m
    pub toast_distance_m: f64,
    pub toast_y_m: f64,
    pub toast_width_m: f64,
    /// SteamVR のダッシュボードに「入退室の履歴」を出す
    pub history_panel: bool,
    pub panel_width_m: f64,
    /// 履歴の置き場所（既定は %LOCALAPPDATA%\vrc-wrist-notifier）
    pub history_dir: PathBuf,
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
            notify_people: "all".into(),
            toast_seconds: 4.0,
            toast_distance_m: 0.8,
            toast_y_m: -0.18,
            toast_width_m: 0.30,
            history_panel: true,
            panel_width_m: 2.6,
            history_dir: PathBuf::new(),
        };
        let mut warn = Vec::new();
        if let Ok(raw) = std::fs::read_to_string(path) {
            let raw = raw.trim_start_matches('\u{feff}');
            match serde_json::from_str::<Value>(raw) {
                Ok(Value::Object(m)) => {
                    for (k, v) in m {
                        match k.as_str() {
                            "notify_people" => c.notify_people = text(&v),
                            "toast_seconds" => c.toast_seconds = num(&v, 4.0).clamp(1.0, 30.0),
                            "toast_distance_m" => c.toast_distance_m = num(&v, 0.8),
                            "toast_y_m" => c.toast_y_m = num(&v, -0.18),
                            "toast_width_m" => c.toast_width_m = num(&v, 0.30),
                            "history_panel" => c.history_panel = truthy(&v),
                            "panel_width_m" => c.panel_width_m = num(&v, 2.6),
                            "history_dir" => c.history_dir = PathBuf::from(text(&v)),
                            // 0.5 までの Frame 側への送り先（もう使わない）
                            "frame_host" | "frame_port" => {}
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
        if c.history_dir.as_os_str().is_empty() {
            c.history_dir = app_dir();
        }
        (c, warn)
    }
}

/// config.json の 1 項目だけを書き換える（ほかの項目と並び順はそのまま。ファイルが無ければ作る）
pub fn save_value(path: &Path, key: &str, value: Value) -> Result<(), String> {
    let mut m = match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<Value>(raw.trim_start_matches('\u{feff}')) {
            Ok(Value::Object(m)) => m,
            _ => return Err(format!("{} を読めないので書き換えません", path.display())),
        },
        Err(_) => serde_json::Map::new(),
    };
    m.insert(key.into(), value);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(&Value::Object(m)).map_err(|e| e.to_string())?;
    // 一時ファイルに書いてから置き換える（途中で止まっても config.json が壊れない）
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, s + "\n").map_err(|e| format!("{} に書けません: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{} に置き換えられません: {e}", path.display()))
}
