// SPDX-License-Identifier: MIT
//! VRChat のログ (output_log_*.txt) を読むだけで、参加・退出・ワールド移動・招待・フレンド申請を見つけて
//! Steam Frame の wrist-toast に UDP (OSC) で送る。VRChat の設定やファイルは変更しない。
//! 0.4 までの PowerShell 版 (vrc-wrist-notifier.ps1) と同じふるまい・同じ送り方（Frame 側は新旧どちらでも読める）。
use crate::config::Config;
use crate::logf;
use chrono::{DateTime, Local, NaiveDateTime, Utc};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const TS_LOG: &str = "%Y.%m.%d %H:%M:%S";
const TS_OUT: &str = "%Y-%m-%d %H:%M:%S";
const TAIL_BYTES: u64 = 512 * 1024;

struct Re {
    line: Regex,
    join: Regex,
    left: Regex,
    room: Regex,
    joining: Regex,
    notif: Regex,
    world_n: Regex,
    auth: Regex,
}
fn re() -> &'static Re {
    static R: OnceLock<Re> = OnceLock::new();
    R.get_or_init(|| Re {
        // 行の形: "2026.10.18 21:03:15 Log        -  [Behaviour] OnPlayerJoined 名前 (usr_...)"
        line: Regex::new(r"^(\d{4}\.\d\d\.\d\d \d\d:\d\d:\d\d) (\w+)\s+-\s+(.*)$").unwrap(),
        join: Regex::new(r"^\[Behaviour\] OnPlayerJoined (.+?)(?: \((usr_[0-9a-fA-F-]+)\))?$").unwrap(),
        left: Regex::new(r"^\[Behaviour\] OnPlayerLeft (.+?)(?: \((usr_[0-9a-fA-F-]+)\))?$").unwrap(),
        room: Regex::new(r"^\[Behaviour\] Entering Room: (.+)$").unwrap(),
        // "Destination set: wrld_xxxx:12345~private(usr_...)" / "Joining wrld_xxxx:..." の wrld_xxxx だけ使う（インスタンスの部分は人の ID が入るので捨てる）
        joining: Regex::new(r"^\[Behaviour\] (?:Joining|Destination set:) (wrld_[0-9a-fA-F-]+)").unwrap(),
        notif: Regex::new(r"^\[API\] Received Notification: <Notification from username:(.*?), sender user id:(usr_[0-9a-fA-F-]+) to .*? of type: ?(\w+), id: ?(not_[0-9a-fA-F-]+)").unwrap(),
        world_n: Regex::new(r"worldName=([^,}]+)").unwrap(),
        auth: Regex::new(r"^\[Behaviour\] User Authenticated: (.+?) \((usr_[0-9a-fA-F-]+)\)").unwrap(),
    })
}

/// 行を (時刻の文字列, レベル, 本文) に分ける。本文の末尾の空白は落とす
fn split_line(line: &str) -> Option<(&str, &str, &str)> {
    let c = re().line.captures(line)?;
    Some((c.get(1)?.as_str(), c.get(2)?.as_str(), c.get(3)?.as_str().trim_end()))
}

fn parse_ts(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, TS_LOG).unwrap_or(NaiveDateTime::MIN)
}

fn secs(a: NaiveDateTime, b: NaiveDateTime) -> f64 {
    (a - b).num_milliseconds() as f64 / 1000.0
}

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// 履歴の重複を除く鍵（Frame 側で同じ鍵は 1 回だけ記録する）
pub fn hist_key(ts: &str, kind: &str, name: &str) -> String {
    format!("{ts}|{kind}|{name}")
}

// ---------------------------------------------------------------- OSC
pub fn osc_packet(address: &str, strings: &[&str]) -> Vec<u8> {
    fn add(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(s.as_bytes());
        let pad = 4 - (s.len() % 4);
        buf.extend(std::iter::repeat(0u8).take(pad));
    }
    let mut buf = Vec::new();
    add(&mut buf, address);
    add(&mut buf, &format!(",{}", "s".repeat(strings.len())));
    for s in strings {
        add(&mut buf, s);
    }
    buf
}

/// 送り先。Dry は送らずに検出結果を out に貯める（テストと --dry-run）
pub enum Sink {
    Dry(Vec<String>),
    Udp { sock: Option<(UdpSocket, SocketAddr)>, error: Option<String> },
}

// ---------------------------------------------------------------- フレンド
/// 裏の取り直しの結果。キーそのものは入れない
#[derive(Debug, Default, Clone)]
pub struct FetchResult {
    pub ok: bool,
    pub ids: Vec<String>,
    pub code: String,
    pub where_: String,
    pub self_id: String,
}
/// (wsl_distro, User-Agent) → 結果。Windows 版では main.rs が渡す（キーを読んで GET /api/1/auth/user）
pub type Fetcher = fn(String, String) -> FetchResult;

struct Unknown {
    uid: String,
    ts: String,
    kind: String,
    name: String,
    world: String,
    wid: String,
}

struct Pending {
    kind: String,
    names: Vec<(String, bool, bool)>, // 名前, フレンド, 自分
    last: NaiveDateTime,
}

/// トレイに出すための今の状態
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// full / errors / unknown / nolog
    pub log: String,
    /// Frame に送っているのと同じ: ok / nokey / bad / none / off
    pub friends: String,
    pub friends_count: usize,
    pub refreshing: bool,
    /// Frame に送れないときの理由
    pub send_error: Option<String>,
    pub world: String,
}

pub struct Watcher {
    pub cfg: Config,
    pub sink: Sink,
    replay: bool,
    pub fetcher: Option<Fetcher>,
    // ワールド
    entered_at: NaiveDateTime,
    world: String,
    world_id: String,
    pending_id: String,
    /// 自分がワールドを出たところ（OnLeftRoom から次の Entering Room まで）。この間の他人の OnPlayerLeft は
    /// VRChat がインスタンスを片付けるときに全員分出すもので、本当に退出したわけではないので、通知も履歴もしない
    leaving: bool,
    seen_notif: HashSet<String>,
    levels: HashMap<String, u64>,
    pending: Vec<(String, Pending)>, // 入れた順（"kind|world" → まとめ）
    // 自分とフレンド
    self_id: String,
    friend_ids: HashSet<String>,
    friends_at: Option<NaiveDateTime>,
    friends_try: Option<Instant>,
    friends_job: Option<Receiver<FetchResult>>,
    friends_key_bad: bool,
    friends_state: String,
    unknown_joins: Vec<Unknown>,
    // 見張り
    last_beat: Option<Instant>,
    last_state: String,
    current: Option<PathBuf>,
    file: Option<File>,
    partial: Vec<u8>,
    last_check: Option<Instant>,
    warned: bool,
}

impl Watcher {
    pub fn new(cfg: Config, dry: bool) -> Watcher {
        let mut w = Watcher {
            cfg,
            sink: if dry { Sink::Dry(Vec::new()) } else { Sink::Udp { sock: None, error: None } },
            replay: false,
            fetcher: None,
            entered_at: NaiveDateTime::MIN,
            world: String::new(),
            world_id: String::new(),
            pending_id: String::new(),
            leaving: false,
            seen_notif: HashSet::new(),
            levels: HashMap::new(),
            pending: Vec::new(),
            self_id: String::new(),
            friend_ids: HashSet::new(),
            friends_at: None,
            friends_try: None,
            friends_job: None,
            friends_key_bad: false,
            friends_state: "none".into(),
            unknown_joins: Vec::new(),
            last_beat: None,
            last_state: String::new(),
            current: None,
            file: None,
            partial: Vec::new(),
            last_check: None,
            warned: false,
        };
        w.load_friends();
        w
    }

    fn dry(&self) -> bool {
        matches!(self.sink, Sink::Dry(_))
    }

    /// --dry-run の出力（テスト用）
    pub fn take_output(&mut self) -> Vec<String> {
        match &mut self.sink {
            Sink::Dry(v) => std::mem::take(v),
            _ => Vec::new(),
        }
    }

    fn print(&mut self, s: String) {
        if let Sink::Dry(v) = &mut self.sink {
            v.push(s);
        }
    }

    // ------------------------------------------------------------ 送信
    fn endpoint(&self) -> Result<SocketAddr, String> {
        let host = self.cfg.frame_host.trim();
        // Frame 側は IPv4 で待っているので、IPv4 のアドレスを選ぶ
        (host, self.cfg.frame_port)
            .to_socket_addrs()
            .map_err(|e| format!("{host} が見つかりません（{e}）"))?
            .find(|a| a.is_ipv4())
            .ok_or_else(|| format!("{host} の IPv4 アドレスが見つかりません。config.json の frame_host に IP を書いてください。"))
    }

    pub fn send_osc(&mut self, address: &str, strings: &[&str]) -> Result<(), String> {
        let pkt = osc_packet(address, strings);
        let ep = match &self.sink {
            Sink::Dry(_) => return Ok(()),
            Sink::Udp { sock: Some((_, ep)), .. } => Ok(*ep),
            Sink::Udp { sock: None, .. } => self.endpoint(),
        };
        let r = ep.and_then(|ep| {
            if let Sink::Udp { sock, .. } = &mut self.sink {
                if sock.is_none() {
                    let s = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("UDP を開けません（{e}）"))?;
                    logf!("送り先: {ep}");
                    *sock = Some((s, ep));
                }
                let (s, ep) = sock.as_ref().unwrap();
                s.send_to(&pkt, ep).map_err(|e| format!("{ep} に送れません（{e}）"))?;
            }
            Ok(())
        });
        if let Sink::Udp { sock, error } = &mut self.sink {
            match &r {
                Ok(()) => {
                    if error.take().is_some() {
                        logf!("Frame に送れるようになりました");
                    }
                }
                Err(e) => {
                    *sock = None; // 次はアドレスを引きなおす
                    if error.as_deref() != Some(e.as_str()) {
                        logf!("送信エラー: {e}");
                    }
                    *error = Some(e.clone());
                }
            }
        }
        r
    }

    /// 通知を送る。ts（yyyy-MM-dd HH:mm:ss）と world を付けると Frame 側の履歴にも残る。quiet なら履歴だけ。
    /// 引数: kind, title, text, ts, world, key, quiet, worldId(0.3.0), 先頭がフレンドなら "1" / 自分なら "self"(0.4.0),
    /// カードに入っているフレンドの名前をタブ区切りで(0.4.1), カードに入っている自分の名前(0.4.1)。古い Frame 側は後ろを読まない
    #[allow(clippy::too_many_arguments)]
    fn send_toast(&mut self, kind: &str, title: &str, text: &str, ts: &str, world: &str, quiet: bool, world_id: &str,
                  friend: bool, friend_names: &[String], is_self: bool, self_name: &str) {
        if self.dry() {
            if !quiet {
                let f = if is_self { "\tself" } else if friend { "\tfriend" } else { "" };
                self.print(format!("{kind}\t{title}\t{text}{f}"));
            }
            return;
        }
        let key = if ts.is_empty() { String::new() } else { hist_key(ts, kind, title) };
        let fr = if is_self { "self" } else if friend { "1" } else { "" };
        let names = friend_names.join("\t");
        let q = if quiet { "quiet" } else { "" };
        if self
            .send_osc("/wrist/notify", &[kind, title, text, ts, world, &key, q, world_id, fr, &names, self_name])
            .is_ok()
        {
            if quiet {
                logf!("履歴だけ送信: [{kind}] {title}");
            } else {
                logf!("送信: [{kind}] {title} / {text}");
            }
        }
    }

    fn simple_toast(&mut self, kind: &str, title: &str, text: &str) {
        self.send_toast(kind, title, text, "", "", false, "", false, &[], false, "");
    }

    /// --test: テスト通知を 1 件
    pub fn send_test(&mut self) -> Result<(), String> {
        self.simple_toast("join", "テスト", "vrc-wrist-notifier から届きました");
        match &self.sink {
            Sink::Udp { error: Some(e), .. } => Err(e.clone()),
            _ => Ok(()),
        }
    }

    // ------------------------------------------------------------ フレンド（0.4.0）
    // VRChat のフレンド一覧（人の ID だけ）を friends.json に置いておき、参加・退出した人がフレンドかを見る。
    // friends.json が friends_refresh_minutes 分（既定 60）より古くなったときだけ取り直す（失敗したときも 1 時間はあける）。
    // 取り直しは裏のスレッドでやるので、通知は止まらない。キーは使うときだけ読み、表示も保存もしない。
    fn load_friends(&mut self) {
        if !self.cfg.friends {
            return;
        }
        let Ok(raw) = std::fs::read_to_string(&self.cfg.friends_file) else { return };
        match serde_json::from_str::<serde_json::Value>(raw.trim_start_matches('\u{feff}')) {
            Ok(j) => {
                self.friend_ids = j["ids"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_str()).filter(|s| s.starts_with("usr_")).map(String::from).collect())
                    .unwrap_or_default();
                if self.self_id.is_empty() {
                    if let Some(s) = j["self_id"].as_str().filter(|s| s.starts_with("usr_")) {
                        self.self_id = s.to_string();
                    }
                }
                self.friends_at = j["fetched_at"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&Local).naive_local());
            }
            Err(e) => logf!("friends.json を読めませんでした（取り直します）: {e}"),
        }
    }

    fn save_friends(&self, ids: &[String], self_id: &str) -> std::io::Result<()> {
        let f = &self.cfg.friends_file;
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d)?;
        }
        let obj = serde_json::json!({
            "fetched_at": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            "count": ids.len(), "self_id": self_id, "ids": ids,
        });
        let mut tmp = f.clone().into_os_string();
        tmp.push(".tmp");
        std::fs::write(&tmp, obj.to_string())?;
        std::fs::rename(&tmp, f)
    }

    fn is_friend(&self, uid: &str) -> bool {
        !uid.is_empty() && self.friend_ids.contains(uid)
    }
    fn is_self(&self, uid: &str) -> bool {
        !uid.is_empty() && uid == self.self_id
    }
    /// 自分の ID: VRChat のログの「User Authenticated」の行（API は要らない）か、フレンド一覧を取ったときの /auth/user の id
    fn update_self(&mut self, body: &str) {
        if !body.contains("User Authenticated: ") {
            return;
        }
        if let Some(c) = re().auth.captures(body) {
            self.self_id = c[2].to_string();
        }
    }

    /// 取り直しを裏で始める（もう動いていれば何もしない）
    pub fn start_friends_refresh(&mut self, why: &str) {
        let Some(fetch) = self.fetcher else { return };
        if !self.cfg.friends || self.friends_job.is_some() || self.dry() || self.replay {
            return;
        }
        self.friends_try = Some(Instant::now());
        let (tx, rx) = std::sync::mpsc::channel();
        let distro = self.cfg.wsl_distro.clone();
        let ua = format!("vrc-wrist-notifier/{} cympfh", crate::VERSION);
        std::thread::spawn(move || {
            let _ = tx.send(fetch(distro, ua));
        });
        self.friends_job = Some(rx);
        logf!("フレンド一覧を取り直しています（{why}）");
    }

    /// トレイの「フレンド一覧を取り直す」: キーを新しくしたあとでも試せるように、キーが効かなかった印も消す
    pub fn manual_refresh(&mut self) {
        self.cfg.friends = true;
        self.friends_key_bad = false;
        self.start_friends_refresh("手動");
    }

    /// 裏の取り直しが終わっていたら受け取る。wait なら終わるまで待つ
    pub fn poll_friends_refresh(&mut self, wait: bool) {
        let Some(rx) = &self.friends_job else { return };
        let r = if wait {
            rx.recv().unwrap_or_else(|_| FetchResult { code: "error".into(), ..Default::default() })
        } else {
            match rx.try_recv() {
                Ok(r) => r,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => FetchResult { code: "error".into(), ..Default::default() },
            }
        };
        self.friends_job = None;
        if r.ok {
            self.friend_ids = r.ids.iter().cloned().collect();
            self.friends_at = Some(now());
            if !r.self_id.is_empty() && self.self_id.is_empty() {
                self.self_id = r.self_id.clone();
            }
            if let Err(e) = self.save_friends(&r.ids, &r.self_id) {
                logf!("friends.json を書けませんでした: {e}");
            }
            self.friends_state = "ok".into();
            logf!("フレンド一覧を取り直しました: {} 人（キーは {} から）", r.ids.len(), r.where_);
            // 取り直す前に参加・退出した人のうち、フレンドだった人は履歴に印を付け直す（同じ鍵なので Frame 側で書き足される）
            let unknown: Vec<Unknown> =
                std::mem::take(&mut self.unknown_joins).into_iter().filter(|u| self.friend_ids.contains(&u.uid)).collect();
            for u in &unknown {
                let key = hist_key(&u.ts, &u.kind, &u.name);
                let _ = self.send_osc("/wrist/history", &[&u.ts, &u.kind, &u.name, &u.world, &key, &u.wid, "1"]);
            }
        } else {
            match r.code.as_str() {
                "nokey" => {
                    self.friends_key_bad = true;
                    self.friends_state = "nokey".into();
                    logf!("VRCHAT_AUTH_KEY が見つからないので、フレンドの色分けはしません（今までどおり動きます）");
                }
                "401" => {
                    self.friends_key_bad = true;
                    self.friends_state = "bad".into();
                    logf!("VRCHAT_AUTH_KEY が効きません（401）。キーを新しくして取り直すまで、今ある一覧を使います");
                }
                code => {
                    self.friends_state = "bad".into();
                    logf!("VRCHAT_AUTH_KEY でフレンド一覧を取れませんでした（{code}）。今ある一覧を使い、1 時間あとにまた試します");
                }
            }
        }
    }

    /// 定期の取り直し（friends.json が friends_refresh_minutes 分より古いとき）。失敗したときも 1 時間はあける
    fn check_friends_due(&mut self) {
        if !self.cfg.friends || self.friends_key_bad || self.friends_job.is_some() {
            return;
        }
        let old = self.friends_at.map_or(true, |t| secs(now(), t) / 60.0 >= self.cfg.friends_refresh_minutes);
        let tried = self.friends_try.is_some_and(|t| t.elapsed() < Duration::from_secs(3600));
        if old && !tried {
            let why = if self.friends_at.is_none() {
                "初めて".to_string()
            } else {
                format!("前回から {} 分以上", self.cfg.friends_refresh_minutes as i64)
            };
            self.start_friends_refresh(&why);
        }
    }

    /// 一覧に無い人を覚えておき、次に取り直したときにフレンドだったら履歴に印を付け直す（API は呼ばない）
    fn note_unknown(&mut self, uid: &str, ts: &str, kind: &str, name: &str) {
        if !self.cfg.friends || uid.is_empty() || self.dry() || self.replay || self.is_self(uid) {
            return;
        }
        if self.unknown_joins.len() < 200 {
            self.unknown_joins.push(Unknown {
                uid: uid.into(), ts: ts.into(), kind: kind.into(), name: name.into(),
                world: self.world.clone(), wid: self.world_id.clone(),
            });
        }
    }

    pub fn friends_summary(&self) -> String {
        let at = self.friends_at.map(|t| t.format(TS_OUT).to_string()).unwrap_or_else(|| "0001-01-01 00:00:00".into());
        format!("friends\t{}\t{}", self.friend_ids.len(), at)
    }

    // ------------------------------------------------------------ 参加・退出をまとめる
    // 履歴には 1 人ずつ残し（quiet）、通知カードだけ「種類 + ワールド」ごとにまとめる。
    // 最後の 1 人から batch_seconds 秒たったら出す。2 人までは 1 人ずつ、3 人以上は「A さんほか N 人が参加」の 1 枚
    fn now_or(&self, log_ts: NaiveDateTime) -> NaiveDateTime {
        if self.replay { log_ts } else { now() }
    }

    fn add_pending(&mut self, kind: &str, name: &str, log_ts: NaiveDateTime, friend: bool, is_self: bool) {
        let now = self.now_or(log_ts);
        let k = format!("{kind}|{}", self.world);
        let i = match self.pending.iter().position(|(x, _)| *x == k) {
            Some(i) => i,
            None => {
                self.pending.push((k, Pending { kind: kind.into(), names: Vec::new(), last: now }));
                self.pending.len() - 1
            }
        };
        let b = &mut self.pending[i].1;
        b.names.push((name.into(), friend, is_self));
        b.last = now;
    }

    fn flush_pending(&mut self, now: NaiveDateTime, all: bool) {
        let batch = self.cfg.batch_seconds;
        let (due, keep): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.pending).into_iter().partition(|(_, b)| all || secs(now, b.last) >= batch);
        self.pending = keep;
        for (_, b) in due {
            let verb = if b.kind == "join" { "参加" } else { "退出" };
            let fnames: Vec<String> = b.names.iter().filter(|n| n.1 && !n.2).map(|n| n.0.clone()).collect();
            let me = b.names.iter().find(|n| n.2);
            let me_name = me.map(|n| n.0.clone()).unwrap_or_default();
            if b.names.len() <= 2 {
                for (name, fr, me) in &b.names {
                    let f = *fr && !*me;
                    let fnl = if f { vec![name.clone()] } else { vec![] };
                    let sn = if *me { name.as_str() } else { "" };
                    self.send_toast(&b.kind, name, &format!("が{verb}しました"), "", "", false, "", f, &fnl, *me, sn);
                }
            } else {
                // まとめカードの先頭には自分、次にフレンドを出す（いなければ最初の人）
                let first = me.or_else(|| b.names.iter().find(|n| n.1)).unwrap_or(&b.names[0]).clone();
                let title = format!("{} さんほか {} 人", first.0, b.names.len() - 1);
                self.send_toast(&b.kind, &title, &format!("が{verb}"), "", "", false, "", first.1 && !first.2, &fnames, first.2, &me_name);
            }
        }
    }

    // ------------------------------------------------------------ ログの解析
    /// ワールドの行なら今のワールドを覚えて "entered"（入った）か "joining"（ID の行）を返す。ほかの行は None。
    /// 今の VRChat は Destination set（ID）→ Entering Room（名前）→ Joining（ID）の順。古いログは Joining が先
    fn update_world(&mut self, body: &str) -> Option<&'static str> {
        if body == "[Behaviour] OnLeftRoom" || body.starts_with("VRCApplication: HandleApplicationQuit") {
            self.leaving = true;
            return Some("left");
        }
        if let Some(c) = re().joining.captures(body) {
            // 入った直後の Joining は、今のワールドの ID（Destination set が無かったとき用）
            let id = c[1].to_string();
            if !self.world.is_empty()
                && (self.world_id.is_empty() || self.world_id == id)
                && self.pending_id.is_empty()
                && body.starts_with("[Behaviour] Joining ")
            {
                self.world_id = id;
            } else {
                self.pending_id = id;
            }
            return Some("joining");
        }
        if let Some(c) = re().room.captures(body) {
            self.world = c[1].trim().to_string();
            self.world_id = std::mem::take(&mut self.pending_id);
            self.leaving = false;
            return Some("entered");
        }
        None
    }

    fn reset_world(&mut self) {
        self.world.clear();
        self.world_id.clear();
        self.pending_id.clear();
        self.leaving = false;
    }

    fn count_level(&mut self, level: &str) {
        *self.levels.entry(level.to_string()).or_insert(0) += 1;
    }

    pub fn handle_line(&mut self, line: &str) {
        let Some((ts_raw, level, body)) = split_line(line) else { return };
        self.count_level(level);
        let ts = parse_ts(ts_raw);
        let tss = ts.format(TS_OUT).to_string();
        if self.replay {
            self.flush_pending(ts, false);
        }
        self.update_self(body);
        match self.update_world(body) {
            Some("joining") | Some("left") => return,
            Some(_) => {
                self.entered_at = ts;
                self.flush_pending(ts, true); // 前のワールドの分は先に出す（ワールドが違えば別のカード）
                let (w, wid) = (self.world.clone(), self.world_id.clone());
                let quiet = !self.cfg.notify_world;
                self.send_toast("world", &w, "ワールドに入りました", &tss, &w, quiet, &wid, false, &[], false, "");
                return;
            }
            None => {}
        }
        for (kind, rx) in [("join", &re().join), ("leave", &re().left)] {
            if let Some(c) = rx.captures(body) {
                let name = c[1].to_string();
                let uid = c.get(2).map_or("", |m| m.as_str()).to_string();
                let me = self.is_self(&uid);
                if kind == "leave" && self.leaving && !me {
                    return; // 自分が出たときの片付け（上の leaving）
                }
                let fr = self.is_friend(&uid) && !me;
                let (w, wid) = (self.world.clone(), self.world_id.clone());
                let text = if kind == "join" { "が参加しました" } else { "が退出しました" };
                self.send_toast(kind, &name, text, &tss, &w, true, &wid, fr, &[], me, "");
                if !fr {
                    self.note_unknown(&uid, &tss, kind, &name);
                }
                let notify = if kind == "join" {
                    // ワールドに入った直後は元からいた人なので通知しない（履歴には残す）
                    let quiet = secs(ts, self.entered_at) < self.cfg.quiet_seconds;
                    !quiet && self.cfg.notify_join
                } else {
                    self.cfg.notify_leave
                };
                if notify {
                    self.add_pending(kind, &name, ts, fr, me);
                }
                return;
            }
        }
        if let Some(c) = re().notif.captures(body) {
            let id = c[4].to_string();
            if !self.seen_notif.insert(id) {
                return;
            }
            let who = c[1].to_string();
            match &c[3] {
                "invite" => {
                    let text = match re().world_n.captures(body) {
                        Some(w) => format!("から招待: {}", w[1].trim()),
                        None => "から招待が届きました".into(),
                    };
                    if self.cfg.notify_invite {
                        self.simple_toast("invite", &who, &text);
                    }
                }
                "requestInvite" => {
                    if self.cfg.notify_invite {
                        self.simple_toast("requestInvite", &who, "から招待のリクエスト");
                    }
                }
                "friendRequest" => {
                    if self.cfg.notify_friend {
                        self.simple_toast("friendRequest", &who, "からフレンド申請");
                    }
                }
                t => {
                    if self.cfg.notify_other {
                        let t = t.to_string();
                        self.simple_toast("info", &who, &format!("から通知 ({t})"));
                    }
                }
            }
        }
    }

    /// VRChat のログ出力の設定を、ログの中身から判断する: full / errors（Error しか出ていない）/ unknown（まだ何も出ていない）
    pub fn log_state(&self) -> &'static str {
        let useful = self.levels.get("Log").copied().unwrap_or(0) + self.levels.get("Debug").copied().unwrap_or(0);
        let total: u64 = self.levels.values().sum();
        if useful > 0 { "full" } else if total > 0 { "errors" } else { "unknown" }
    }

    fn warn_if_errors_only(&self) {
        if self.log_state() == "errors" {
            logf!("VRChat のログに Error しか出ていません。参加・退出を見つけるには、VRChat の クイックメニュー → 設定 → Debug（デバッグ）のログ出力を「Full」にしてください。");
        }
    }

    fn friends_wire(&self) -> String {
        if !self.cfg.friends {
            "off".into()
        } else if self.friends_state == "none" && !self.friend_ids.is_empty() {
            "ok".into()
        } else {
            self.friends_state.clone()
        }
    }

    /// Frame 側に「動いています」を知らせる（30 秒ごと、状態が変わったときはすぐ）。古い Frame 側は知らないアドレスとして捨てるだけ。
    /// 引数: 版、ログ出力の状態（full / errors / unknown / nolog）、キーの状態（ok / nokey / bad / none / off）
    fn send_heartbeat(&mut self, state: &str) {
        let key2 = format!("{state}|{}", self.friends_state);
        if key2 == self.last_state && self.last_beat.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            return;
        }
        self.last_beat = Some(Instant::now());
        self.last_state = key2;
        let fs = self.friends_wire();
        let _ = self.send_osc("/wrist/heartbeat", &[crate::VERSION, state, &fs]);
    }

    // ------------------------------------------------------------ 過去のログ・今のワールド
    pub fn log_files(&self) -> Vec<PathBuf> {
        let mut v: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&self.cfg.log_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("output_log_") && n.ends_with(".txt")
            })
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        v.sort();
        v.into_iter().map(|x| x.1).collect()
    }

    pub fn newest_log(&self) -> Option<PathBuf> {
        self.log_files().pop()
    }

    /// 今ある VRChat のログ（古い順）から、参加・退出・ワールドを履歴として送る。通知は出さない
    pub fn backfill(&mut self) {
        let files = self.log_files();
        let mut count = 0usize;
        for f in &files {
            self.reset_world(); // ワールドはログのファイルごと（VRChat を起動しなおすと新しいファイルになる）
            let Ok(file) = File::open(f) else { continue };
            for line in lines_lossy(BufReader::new(file)) {
                let Some((ts_raw, _, body)) = split_line(&line) else { continue };
                self.update_self(body);
                let (kind, name, uid) = match self.update_world(body) {
                    Some("joining") | Some("left") => continue,
                    Some(_) => ("world", self.world.clone(), String::new()),
                    None => {
                        if let Some(c) = re().join.captures(body) {
                            ("join", c[1].to_string(), c.get(2).map_or("", |m| m.as_str()).to_string())
                        } else if let Some(c) = re().left.captures(body) {
                            ("leave", c[1].to_string(), c.get(2).map_or("", |m| m.as_str()).to_string())
                        } else {
                            continue;
                        }
                    }
                };
                if kind == "leave" && self.leaving && !self.is_self(&uid) {
                    continue; // 自分が出たときの片付け
                }
                let ts = parse_ts(ts_raw).format(TS_OUT).to_string();
                let fr = if self.is_self(&uid) { "self" } else if self.is_friend(&uid) { "1" } else { "" };
                if self.dry() {
                    let mark = match fr { "self" => "\tself", "1" => "\tfriend", _ => "" };
                    let s = format!("history\t{ts}\t{kind}\t{name}\t{}\t{}{mark}", self.world, self.world_id);
                    self.print(s);
                } else {
                    // 6 個目のワールド ID は 0.3.0 から、7 個目のフレンドの印は 0.4.0 から（古い Frame 側は読まない）
                    let (w, wid) = (self.world.clone(), self.world_id.clone());
                    let key = hist_key(&ts, kind, &name);
                    let _ = self.send_osc("/wrist/history", &[&ts, kind, &name, &w, &key, &wid, fr]);
                    count += 1;
                    if count % 50 == 0 {
                        std::thread::sleep(Duration::from_millis(20)); // 一度に送りすぎない
                    }
                }
            }
        }
        if !self.dry() {
            logf!("過去のログから履歴を {count} 件送りました（{} ファイル。重複は Frame 側で除きます）", files.len());
        }
        self.reset_world();
    }

    /// ログのファイルを頭から読んで、最後に入ったワールドを覚える（見張りを途中から始めたとき用）。
    /// 速さのため、ワールドの行と自分の行だけ正規表現にかける
    pub fn read_current_world(&mut self, path: &Path) {
        self.reset_world();
        let Ok(file) = File::open(path) else { return };
        for l in lines_lossy(BufReader::with_capacity(65536, file)) {
            if l.contains("[Behaviour] Entering Room: ")
                || l.contains("[Behaviour] Joining wrld_")
                || l.contains("[Behaviour] Destination set: wrld_")
                || l.contains("User Authenticated: ")
            {
                if let Some((ts, _, body)) = split_line(&l) {
                    self.update_self(body);
                    if self.update_world(body) == Some("entered") {
                        self.entered_at = parse_ts(ts);
                    }
                }
            }
        }
    }

    /// --show-world の 1 行
    pub fn world_line(&self) -> String {
        if self.world.is_empty() {
            "world\t\t\t".into()
        } else {
            format!("world\t{}\t{}\t{}", self.world, self.world_id, self.entered_at.format(TS_OUT))
        }
    }

    /// --status: いちばん新しいログの末尾 512KB から、ログ出力の設定を見る
    pub fn status_line(&mut self) -> String {
        let Some(newest) = self.newest_log() else { return "log\tnolog".into() };
        if let Ok(mut f) = File::open(&newest) {
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            let _ = f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)));
            for l in lines_lossy(BufReader::new(f)) {
                if let Some((_, level, _)) = split_line(&l) {
                    self.count_level(level);
                }
            }
        }
        let name = newest.file_name().unwrap_or_default().to_string_lossy().to_string();
        format!("log\t{}\t{name}", self.log_state())
    }

    /// --replay <ファイル>: ログを頭から流す（時刻はログの時刻を使う）
    pub fn replay(&mut self, path: &Path) -> std::io::Result<()> {
        self.replay = true;
        let file = File::open(path)?;
        for line in lines_lossy(BufReader::new(file)) {
            self.handle_line(&line);
        }
        self.flush_pending(NaiveDateTime::MAX, true);
        if !self.dry() {
            self.warn_if_errors_only();
        }
        Ok(())
    }

    // ------------------------------------------------------------ 見張り
    /// 見張りを始めるときに 1 回
    pub fn start(&mut self) {
        logf!("vrc-wrist-notifier {} を開始（ログの場所: {}）", crate::VERSION, self.cfg.log_dir.display());
        if self.cfg.friends {
            if !self.friend_ids.is_empty() {
                let at = self.friends_at.map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                logf!("フレンド一覧: {} 人（{at} に取得）", self.friend_ids.len());
            }
            // 起動時に 1 回だけ、キーが使えるかを確かめる（キーが無ければ API は呼ばない。あれば一覧の取り直しを兼ねて 1 回だけ呼ぶ）
            self.start_friends_refresh("起動時の確認");
        }
        if self.cfg.backfill_history {
            self.backfill();
        }
    }

    /// 0.5 秒ごとに呼ぶ
    pub fn tick(&mut self) {
        if self.last_check.map_or(true, |t| t.elapsed() >= Duration::from_secs(5)) {
            self.last_check = Some(Instant::now());
            if let Some(newest) = self.newest_log() {
                if self.current.as_ref() != Some(&newest) {
                    self.open_log(newest);
                }
            }
        }
        let state = if self.file.is_some() { self.log_state() } else { "nolog" };
        self.send_heartbeat(state);
        if self.file.is_some() {
            self.read_appended();
        } else if !self.warned {
            logf!("VRChat のログがまだありません。VRChat を起動すると読み始めます。");
            self.warned = true;
        }
        self.flush_pending(now(), false);
        self.poll_friends_refresh(false);
        self.check_friends_due();
    }

    fn open_log(&mut self, path: PathBuf) {
        let is_first = self.current.is_none();
        let Ok(mut f) = File::open(&path) else { return };
        self.current = Some(path.clone());
        if is_first {
            // 途中から見はじめたときも、今いるワールドがわかるように
            self.read_current_world(&path);
            if !self.world.is_empty() {
                logf!("今いるワールド: {}", self.world);
            }
            // 起動前のことは通知しない。Debug 設定の確認と、通知の二重表示を防ぐため、末尾 512KB だけ読む
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            let _ = f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)));
            let mut buf = Vec::new();
            let _ = (&mut f).take(len).read_to_end(&mut buf);
            for l in String::from_utf8_lossy(&buf).lines() {
                if let Some((_, level, body)) = split_line(l) {
                    self.count_level(level);
                    if let Some(c) = re().notif.captures(body) {
                        self.seen_notif.insert(c[4].to_string());
                    }
                }
            }
            let _ = f.seek(SeekFrom::Start(len));
            self.warn_if_errors_only();
            self.warned = true;
        } else {
            self.reset_world(); // 新しいログ = VRChat を起動しなおした
            self.levels.clear();
        }
        self.file = Some(f);
        self.partial.clear();
        logf!("読んでいるログ: {}", path.file_name().unwrap_or_default().to_string_lossy());
    }

    fn read_appended(&mut self) {
        let mut chunk = Vec::new();
        if let Some(f) = self.file.as_mut() {
            let _ = f.read_to_end(&mut chunk);
        }
        if chunk.is_empty() {
            return;
        }
        self.partial.extend_from_slice(&chunk);
        // 行の途中（と、UTF-8 の途中）は次に回す
        let Some(end) = self.partial.iter().rposition(|&b| b == b'\n') else { return };
        let rest = self.partial.split_off(end + 1);
        let done = std::mem::replace(&mut self.partial, rest);
        for l in String::from_utf8_lossy(&done).lines() {
            self.handle_line(l);
        }
    }

    pub fn status(&self) -> Status {
        Status {
            log: if self.file.is_some() { self.log_state().into() } else { "nolog".into() },
            friends: self.friends_wire(),
            friends_count: self.friend_ids.len(),
            refreshing: self.friends_job.is_some(),
            send_error: match &self.sink {
                Sink::Udp { error, .. } => error.clone(),
                _ => None,
            },
            world: self.world.clone(),
        }
    }
}

/// UTF-8 として読めない行があっても止まらない
fn lines_lossy<R: BufRead>(mut r: R) -> impl Iterator<Item = String> {
    std::iter::from_fn(move || {
        let mut buf = Vec::new();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                while matches!(buf.last(), Some(b'\n' | b'\r')) {
                    buf.pop();
                }
                Some(String::from_utf8_lossy(&buf).into_owned())
            }
        }
    })
}
