// SPDX-License-Identifier: MIT
//! vrc-wrist-notifier。引数なしで起動すると、窓を出さずにシステムトレイに入って見張り、SteamVR に通知を出す。
//!
//!   vrc-wrist-notifier.exe                         トレイに入って見張る（2 つ目は起動しない）
//!   vrc-wrist-notifier.exe --replay <ログ> --dry-run   検出結果だけ表示
//!   vrc-wrist-notifier.exe --backfill [--dry-run]  過去のログから履歴に足して終わる
//!   vrc-wrist-notifier.exe --dump-png <フォルダ> [--history-dir <フォルダ>]
//!                                                  通知のカードと履歴のパネルを PNG に描いて終わる（見た目の確認用）
//!   vrc-wrist-notifier.exe --show-world            いちばん新しいログから、今いるワールドを表示するだけ
//!   vrc-wrist-notifier.exe --status                VRChat のログ出力が Full になっているかを表示するだけ
//!   vrc-wrist-notifier.exe --refresh-friends       フレンド一覧を今すぐ取り直して、件数だけ表示
//!   どれにも --config <config.json> を付けられる
//! PowerShell から表示を見るときは `| Out-String` などを付ける（トレイのアプリなので、付けないと待たずに戻る）。
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use vwn::config::{self, Config};
use vwn::watcher::{Out, Watcher};
use vwn::{logf, VERSION};

#[cfg(windows)]
mod friends_api;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod d3d;
#[cfg(windows)]
mod vr;

#[derive(Default)]
struct Args {
    config: Option<PathBuf>,
    dump_png: Option<PathBuf>,
    history_dir: Option<PathBuf>,
    replay: Option<PathBuf>,
    backfill: bool,
    show_world: bool,
    status: bool,
    refresh_friends: bool,
    dry_run: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args_os().skip(1);
    while let Some(x) = it.next() {
        let s = x.to_string_lossy().to_string();
        // PowerShell 版の書き方（-Replay, -DryRun など）も受け付ける
        match s.trim_start_matches('-').to_ascii_lowercase().replace('-', "").as_str() {
            "config" => a.config = Some(it.next().ok_or("--config の後にファイルがありません")?.into()),
            "historydir" => a.history_dir = Some(it.next().ok_or("--history-dir の後にフォルダがありません")?.into()),
            "dumppng" => a.dump_png = Some(it.next().ok_or("--dump-png の後にフォルダがありません")?.into()),
            "replay" => a.replay = Some(it.next().ok_or("--replay の後にファイルがありません")?.into()),
            "backfill" => a.backfill = true,
            "showworld" => a.show_world = true,
            "status" => a.status = true,
            "refreshfriends" => a.refresh_friends = true,
            "dryrun" => a.dry_run = true,
            "version" => {
                out(&format!("vrc-wrist-notifier {VERSION}"));
                std::process::exit(0);
            }
            _ => return Err(format!("知らない引数: {s}")),
        }
    }
    Ok(a)
}

/// コマンドとして使ったときの表示（トレイのアプリなので、コンソールから起動されたらそこに書く）
fn out(s: &str) {
    #[cfg(windows)]
    attach_console();
    use std::io::Write;
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{s}");
    let _ = o.flush();
}

#[cfg(windows)]
fn attach_console() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        // 出力がパイプやファイルに向いていればそのまま。向いていなければ、起動したコンソールにつなぐ
        use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h.is_null() || h as isize == -1 {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    });
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            out(&e);
            std::process::exit(2);
        }
    };
    let cli = args.dump_png.is_some() || args.replay.is_some() || args.backfill || args.show_world || args.status || args.refresh_friends;
    if !cli {
        #[cfg(windows)]
        {
            vwn::log::to_file(config::app_dir().join("vrc-wrist-notifier.log"));
        }
    }
    let cfg_path = args.config.clone().unwrap_or_else(config::default_path);
    let (cfg, warns) = Config::load(&cfg_path);
    for w in &warns {
        logf!("{w}");
    }
    if let Some(dir) = &args.dump_png {
        if let Err(e) = dump_png(dir, args.history_dir.clone()) {
            out(&e);
            std::process::exit(1);
        }
        return;
    }
    if !cli {
        #[cfg(windows)]
        tray::run(cfg, cfg_path);
        #[cfg(not(windows))]
        out("トレイと SteamVR のオーバーレイは Windows だけです");
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel::<Out>();
    let live = !args.dry_run && args.backfill;
    let hist_dir = cfg.history_dir.clone();
    let mut w = Watcher::new(cfg, live.then_some(tx));
    #[cfg(windows)]
    {
        w.fetcher = Some(friends_api::fetch);
    }

    if args.refresh_friends {
        w.manual_refresh();
        w.poll_friends_refresh(true);
        out(&w.friends_summary());
        return;
    }
    if args.backfill {
        w.backfill();
        w.take_output().iter().for_each(|l| out(l));
        if live {
            drop(w);
            let mut h = vwn::history::History::new(hist_dir);
            h.load();
            let n = rx.try_iter().filter(|o| matches!(o, Out::History(e) if h.add(e.clone()))).count();
            out(&format!("履歴に {n} 件足しました（{}）", h.path().display()));
        }
        return;
    }
    if args.status {
        out(&w.status_line());
        return;
    }
    if args.show_world {
        let Some(newest) = w.newest_log() else {
            out(&format!("VRChat のログが見つかりません（{}）", w.cfg.log_dir.display()));
            std::process::exit(1);
        };
        w.read_current_world(&newest);
        out(&w.world_line());
        return;
    }
    if let Some(p) = &args.replay {
        if let Err(e) = w.replay(p) {
            out(&format!("{} を読めません: {e}", p.display()));
            std::process::exit(1);
        }
        w.take_output().iter().for_each(|l| out(l));
    }
}

/// 見た目の確認用: 通知のカードを何種類かと、履歴のパネル（今の履歴）を PNG に
fn dump_png(dir: &std::path::Path, history_dir: Option<PathBuf>) -> Result<(), String> {
    use vwn::toast::{render, Toast};
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut fr = Toast::new("join", "Alice", "が参加しました");
    fr.fr = true;
    let cards = [
        ("toast-join", Toast::new("join", "Bob", "が参加しました")),
        ("toast-friend", fr),
        ("toast-leave", Toast::new("leave", "Bob さんほか 3 人", "が退出")),
        ("toast-world", Toast::new("world", "The Black Cat", "ワールドに入りました")),
        ("toast-invite", Toast::new("invite", "Alice", "から招待: The Black Cat")),
        ("toast-alert", Toast::new("alert", "テスト", "急ぎの通知の色")),
    ];
    for (name, t) in cards {
        render(&t).save_png(&dir.join(format!("{name}.png")))?;
    }
    let (cfg, _) = Config::load(&config::default_path());
    let mut h = vwn::history::History::new(history_dir.unwrap_or_else(|| cfg.history_dir.clone()));
    h.load();
    let mut p = vwn::panel::HistoryPanel::new();
    p.set_notify(vwn::toast::Notify::from(&cfg.notify_people));
    p.set_status(&vwn::panel::PanelStatus { log: "full".into(), friends: "ok".into() });
    p.render(h.events()).save_png(&dir.join("panel.png"))?;
    out(&format!("{} に書きました（履歴 {} 件）", dir.display(), h.events().len()));
    Ok(())
}
