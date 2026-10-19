// SPDX-License-Identifier: MIT
//! vrc-wrist-notifier（Windows 側）。引数なしで起動すると、窓を出さずにシステムトレイに入って見張る。
//!
//!   vrc-wrist-notifier.exe                         トレイに入って見張る（2 つ目は起動しない）
//!   vrc-wrist-notifier.exe --test                  テスト通知を 1 件送る
//!   vrc-wrist-notifier.exe --replay <ログ> --dry-run   送らずに検出結果だけ表示
//!   vrc-wrist-notifier.exe --backfill [--dry-run]  過去のログから履歴だけ送って終わる
//!   vrc-wrist-notifier.exe --show-world            いちばん新しいログから、今いるワールドを表示するだけ
//!   vrc-wrist-notifier.exe --status                VRChat のログ出力が Full になっているかを表示するだけ
//!   vrc-wrist-notifier.exe --refresh-friends       フレンド一覧を今すぐ取り直して、件数だけ表示
//!   どれにも --config <config.json> を付けられる
//! PowerShell から表示を見るときは `| Out-String` などを付ける（トレイのアプリなので、付けないと待たずに戻る）。
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use vwn::config::{self, Config};
use vwn::watcher::Watcher;
use vwn::{logf, VERSION};

#[cfg(windows)]
mod friends_api;
#[cfg(windows)]
mod tray;

#[derive(Default)]
struct Args {
    config: Option<PathBuf>,
    test: bool,
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
            "test" => a.test = true,
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
    let cli = args.test || args.replay.is_some() || args.backfill || args.show_world || args.status || args.refresh_friends;
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
    let mut w = Watcher::new(cfg, args.dry_run);
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
    if args.test {
        if let Err(e) = w.send_test() {
            out(&format!("送れませんでした: {e}"));
            std::process::exit(1);
        }
        if args.dry_run {
            w.take_output().iter().for_each(|l| out(l));
        }
        return;
    }
    if args.backfill {
        w.backfill();
        w.take_output().iter().for_each(|l| out(l));
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
        return;
    }

    #[cfg(windows)]
    tray::run(w);
    #[cfg(not(windows))]
    {
        // Windows 以外（開発用）: トレイなしで見張る
        w.start();
        loop {
            w.tick();
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}
