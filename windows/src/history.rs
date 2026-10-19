// SPDX-License-Identifier: MIT
//! 入退室の履歴: %LOCALAPPDATA%\vrc-wrist-notifier\history.jsonl に 1 行 1 件で追記する（0.5 までの Frame 側と同じ形式なので、
//! Frame の ~/.local/state/vrc-wrist-notifier/history.jsonl をそのまま置けば読める）。人の名前が入るので、どこにも送らない。
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistEvent {
    /// "YYYY-MM-DD HH:MM:SS"（PC の時刻）
    pub t: String,
    /// join / leave / world
    pub kind: String,
    /// 人の名前、world のときはワールド名
    pub name: String,
    /// その時いたワールド
    pub world: String,
    /// 重複を除くための鍵
    pub key: String,
    /// ワールドの ID（wrld_...、わからなければ空）
    pub wid: String,
    pub fr: bool,
    pub me: bool,
}

impl HistEvent {
    pub fn to_json(&self) -> String {
        let mut o = json!({ "t": self.t, "k": self.kind, "n": self.name, "w": self.world, "key": self.key });
        if !self.wid.is_empty() {
            o["wid"] = json!(self.wid);
        }
        if self.fr {
            o["f"] = json!("1");
        }
        if self.me {
            o["s"] = json!("1");
        }
        o.to_string()
    }
    /// ワールド（w）とその ID（wid）は無くても読む（古い版の行）
    pub fn from_json(line: &str) -> Option<HistEvent> {
        let v: Value = serde_json::from_str(line).ok()?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
        let (t, kind, name) = (s("t")?, s("k")?, s("n")?);
        let key = s("key").unwrap_or_else(|| format!("{t}|{kind}|{name}"));
        Some(HistEvent {
            world: s("w").unwrap_or_default(),
            wid: s("wid").unwrap_or_default(),
            fr: s("f").as_deref() == Some("1"),
            me: s("s").as_deref() == Some("1"),
            t, kind, name, key,
        })
    }
}

pub struct History {
    dir: PathBuf,
    events: VecDeque<HistEvent>, // 古い順
    keys: HashSet<String>,
    version: u64,
}

const MAX_BYTES: u64 = 2 * 1024 * 1024; // これを超えたら history.1.jsonl に回す（1 世代だけ残す）
const MAX_IN_MEMORY: usize = 20000;

impl History {
    pub fn new(dir: PathBuf) -> History {
        History { dir, events: VecDeque::new(), keys: HashSet::new(), version: 0 }
    }
    pub fn path(&self) -> PathBuf {
        self.dir.join("history.jsonl")
    }
    fn old_path(&self) -> PathBuf {
        self.dir.join("history.1.jsonl")
    }
    pub fn events(&self) -> &VecDeque<HistEvent> {
        &self.events
    }
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn load(&mut self) {
        self.events.clear();
        self.keys.clear();
        self.load_file(&self.old_path());
        self.load_file(&self.path());
        self.version += 1;
    }

    fn load_file(&mut self, p: &Path) {
        // バイトのまま行に分け、行ごとに UTF-8 にする（途中で切れた行・名前の途中で切れた行があっても、その行だけ読み飛ばす）
        let Ok(bytes) = std::fs::read(p) else { return };
        for raw in bytes.split(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(raw);
            let Some(e) = HistEvent::from_json(line.trim_end_matches('\r')) else { continue };
            if !self.keys.contains(&e.key) {
                self.insert_sorted(e);
            } else if let Some(old) = self.find(&e.key) {
                if fills_in(old, &e) {
                    merge(old, &e);
                }
            }
        }
    }

    /// 追加できたら true（同じ鍵がもうあれば false）。同じ鍵でも、前の記録に無いワールド・フレンド・自分の印があれば書き足して true
    pub fn add(&mut self, mut e: HistEvent) -> bool {
        if !matches!(e.kind.as_str(), "join" | "leave" | "world") {
            return false;
        }
        if e.t.is_empty() {
            e.t = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        }
        if e.key.is_empty() {
            e.key = format!("{}|{}|{}", e.t, e.kind, e.name);
        }
        if self.keys.contains(&e.key) {
            let Some(old) = self.find(&e.key) else { return false };
            if !fills_in(old, &e) {
                return false;
            }
            merge(old, &e);
            let old = old.clone();
            self.append(&old); // 読み直したときは後の行が勝つ
            self.version += 1;
            return true;
        }
        self.append(&e);
        self.insert_sorted(e);
        self.version += 1;
        true
    }

    fn find(&mut self, key: &str) -> Option<&mut HistEvent> {
        self.events.iter_mut().rev().find(|e| e.key == key)
    }

    fn insert_sorted(&mut self, e: HistEvent) {
        self.keys.insert(e.key.clone());
        // ほとんどは末尾。過去の分（起動時の読み込み直し）は時刻の位置に入れる
        let mut i = self.events.len();
        while i > 0 && self.events[i - 1].t > e.t {
            i -= 1;
        }
        self.events.insert(i, e);
        while self.events.len() > MAX_IN_MEMORY {
            self.events.pop_front();
        }
    }

    fn append(&self, e: &HistEvent) {
        let _ = std::fs::create_dir_all(&self.dir);
        if std::fs::metadata(self.path()).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
            let _ = std::fs::rename(self.path(), self.old_path()); // 前の世代は置き換わる（容量の上限）
        }
        let Ok(mut f) = std::fs::OpenOptions::new().create(true).read(true).append(true).open(self.path()) else { return };
        // 前の行が途中で切れていたら（改行で終わっていなければ）、改行を足してから書く
        let mut rec = format!("{}\n", e.to_json());
        if let Ok(len) = f.seek(SeekFrom::End(0)) {
            if len > 0 {
                let mut last = [0u8; 1];
                if f.seek(SeekFrom::Start(len - 1)).is_ok() && f.read_exact(&mut last).is_ok() && last[0] != b'\n' {
                    rec.insert(0, '\n');
                }
            }
        }
        // 1 件を 1 回で書く（追記モードなので、同時に書いても行が混ざらない）
        let _ = f.write_all(rec.as_bytes());
    }
}

/// b が a に無いワールドの情報や、フレンド・自分の印を持っているか
fn fills_in(a: &HistEvent, b: &HistEvent) -> bool {
    (a.world.is_empty() && !b.world.is_empty())
        || (a.wid.is_empty() && !b.wid.is_empty() && (a.world.is_empty() || a.world == b.world))
        || (!a.fr && b.fr)
        || (!a.me && b.me)
}
fn merge(a: &mut HistEvent, b: &HistEvent) {
    if a.world.is_empty() {
        a.world = b.world.clone();
    }
    if a.wid.is_empty() && a.world == b.world {
        a.wid = b.wid.clone();
    }
    a.fr |= b.fr;
    a.me |= b.me;
}
