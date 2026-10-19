// SPDX-License-Identifier: MIT
//! 動作ログ。トレイのアプリには窓が無いので、%LOCALAPPDATA%\vrc-wrist-notifier\vrc-wrist-notifier.log に書く
//! （1 MB を超えたら .old に回す）。キーは決して書かない。
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static SINK: Mutex<Option<(PathBuf, File)>> = Mutex::new(None);

pub fn to_file(path: PathBuf) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(m) = std::fs::metadata(&path) {
        if m.len() > 1_000_000 {
            let _ = std::fs::rename(&path, path.with_extension("log.old"));
        }
    }
    if let Ok(f) = OpenOptions::new().create(true).append(true).open(&path) {
        *SINK.lock().unwrap() = Some((path, f));
    }
}

pub fn path() -> Option<PathBuf> {
    SINK.lock().unwrap().as_ref().map(|(p, _)| p.clone())
}

pub fn log(msg: &str) {
    let line = format!("{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    let mut g = SINK.lock().unwrap();
    match g.as_mut() {
        Some((_, f)) => {
            let _ = writeln!(f, "{line}");
        }
        None => eprintln!("{line}"),
    }
}

#[macro_export]
macro_rules! logf {
    ($($a:tt)*) => { $crate::log::log(&format!($($a)*)) };
}
