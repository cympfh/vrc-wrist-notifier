// SPDX-License-Identifier: MIT
//! 見張り（watcher.rs）から届いたものを、履歴と通知の待ち行列に振り分ける。SteamVR には触らない（それは vr.rs）。
use crate::config::{self, Config};
use crate::history::History;
use crate::logf;
use crate::panel::{HistoryPanel, PanelStatus};
use crate::toast::{self, Notify, Toast};
use crate::watcher::Out;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const MAX_QUEUE: usize = 8;
const PER_SECOND: u32 = 10;

pub struct Hub {
    pub history: History,
    pub panel: HistoryPanel,
    pub queue: VecDeque<Toast>,
    cfg_path: PathBuf,
    window_start: Option<Instant>,
    window_count: u32,
}

impl Hub {
    pub fn new(cfg: &Config, cfg_path: PathBuf) -> Hub {
        let mut history = History::new(cfg.history_dir.clone());
        history.load();
        let mut panel = HistoryPanel::new();
        panel.set_notify(Notify::from(&cfg.notify_people));
        Hub { history, panel, queue: VecDeque::new(), cfg_path, window_start: None, window_count: 0 }
    }

    pub fn handle(&mut self, o: Out) {
        match o {
            Out::History(e) => {
                self.history.add(e);
            }
            Out::Toast { toast: t, friends, self_name } => {
                let mode = self.panel.notify();
                let shown = toast::filter_people(&t, mode, &friends, &self_name);
                if shown.is_empty() {
                    logf!("通知設定が「{}」なので出しません: [{}] {}", mode.name(), t.kind, t.title);
                }
                for one in shown {
                    let now = Instant::now();
                    if self.window_start.map_or(true, |s| now - s > Duration::from_secs(1)) {
                        self.window_start = Some(now);
                        self.window_count = 0;
                    }
                    self.window_count += 1;
                    if self.window_count > PER_SECOND || self.queue.len() >= MAX_QUEUE {
                        logf!("多すぎるので捨てました: [{}] {}", one.kind, one.title);
                    } else {
                        self.queue.push_back(one);
                    }
                }
            }
        }
    }

    /// パネルで「通知設定」が選び直されたら config.json に書く
    pub fn save_notify_if_changed(&mut self) {
        if !self.panel.take_notify_change() {
            return;
        }
        let v = self.panel.notify().name();
        match config::save_value(&self.cfg_path, "notify_people", v.into()) {
            Ok(()) => logf!("通知設定を「{v}」にしました"),
            Err(e) => logf!("通知設定を保存できませんでした: {e}"),
        }
    }

    pub fn set_status(&mut self, log: &str, friends: &str) -> bool {
        self.panel.set_status(&PanelStatus { log: log.into(), friends: friends.into() })
    }
}
