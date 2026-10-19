// SPDX-License-Identifier: MIT
//! vrc-wrist-notifier（Windows 側）の中身。トレイなど Windows だけの部分は main.rs。
//! ここはどの OS でも動くので、テスト（cargo test）は WSL / Linux でも回せる。
pub mod auth;
pub mod config;
pub mod draw;
pub mod history;
pub mod hub;
pub mod panel;
pub mod toast;
pub mod upload;
pub mod log;
pub mod watcher;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
