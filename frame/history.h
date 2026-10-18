// SPDX-License-Identifier: MIT
// 入退室の履歴: ~/.local/state/vrc-wrist-notifier/history.jsonl に 1 行 1 件で追記する。
// 人の名前が入るので、このファイルは Frame の中だけに置く（リポジトリには入れない）。
#pragma once

#include <sys/stat.h>

#include <cstdio>
#include <ctime>
#include <deque>
#include <fstream>
#include <string>
#include <unordered_set>
#include <vector>

struct HistEvent {
    std::string t;      // "YYYY-MM-DD HH:MM:SS"（その PC の時刻 = 日本時間）
    std::string kind;   // join / leave / world
    std::string name;   // 人の名前、world のときはワールド名
    std::string world;  // その時いたワールド
    std::string key;    // 重複を除くための鍵
    std::string wid;    // ワールドの ID（wrld_...、0.3.0 から。わからなければ空）
};

namespace hist {

inline std::string json_escape(const std::string& s) {
    std::string o;
    for (unsigned char c : s) {
        if (c == '"' || c == '\\') { o += '\\'; o += char(c); }
        else if (c < 0x20) { char b[8]; snprintf(b, sizeof b, "\\u%04x", c); o += b; }
        else o += char(c);
    }
    return o;
}

inline void put_utf8(std::string& o, unsigned cp) {
    if (cp < 0x80) o += char(cp);
    else if (cp < 0x800) { o += char(0xC0 | (cp >> 6)); o += char(0x80 | (cp & 0x3F)); }
    else if (cp < 0x10000) { o += char(0xE0 | (cp >> 12)); o += char(0x80 | ((cp >> 6) & 0x3F)); o += char(0x80 | (cp & 0x3F)); }
    else { o += char(0xF0 | (cp >> 18)); o += char(0x80 | ((cp >> 12) & 0x3F)); o += char(0x80 | ((cp >> 6) & 0x3F)); o += char(0x80 | (cp & 0x3F)); }
}

// {"k":"v",...} の平たい文字列だけの JSON を読む（自分で書いた形式だけ読めればよい）
inline bool json_get(const std::string& line, const std::string& field, std::string& out) {
    std::string pat = "\"" + field + "\":\"";
    size_t i = line.find(pat);
    if (i == std::string::npos) return false;
    i += pat.size();
    out.clear();
    while (i < line.size() && line[i] != '"') {
        char c = line[i++];
        if (c != '\\') { out += c; continue; }
        if (i >= line.size()) return false;
        char e = line[i++];
        if (e == 'u' && i + 4 <= line.size()) {
            unsigned cp = std::stoul(line.substr(i, 4), nullptr, 16);
            i += 4;
            if (cp >= 0xD800 && cp < 0xDC00 && i + 6 <= line.size() && line[i] == '\\' && line[i + 1] == 'u') {
                unsigned lo = std::stoul(line.substr(i + 2, 4), nullptr, 16);
                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                i += 6;
            }
            put_utf8(out, cp);
        } else if (e == 'n') out += '\n';
        else if (e == 't') out += '\t';
        else out += e;
    }
    return i < line.size();
}

inline std::string to_json(const HistEvent& e) {
    std::string o = "{\"t\":\"" + json_escape(e.t) + "\",\"k\":\"" + json_escape(e.kind) + "\",\"n\":\"" + json_escape(e.name) +
                    "\",\"w\":\"" + json_escape(e.world) + "\",\"key\":\"" + json_escape(e.key) + "\"";
    if (!e.wid.empty()) o += ",\"wid\":\"" + json_escape(e.wid) + "\"";
    return o + "}";
}

// ワールド（w）とその ID（wid）は無くても読む（古い版の行）
inline bool from_json(const std::string& line, HistEvent& e) {
    if (!json_get(line, "t", e.t) || !json_get(line, "k", e.kind) || !json_get(line, "n", e.name)) return false;
    if (!json_get(line, "w", e.world)) e.world.clear();
    if (!json_get(line, "wid", e.wid)) e.wid.clear();
    if (!json_get(line, "key", e.key)) e.key = e.t + "|" + e.kind + "|" + e.name;
    return true;
}

inline std::string now_local() {
    char b[32];
    time_t t = time(nullptr);
    strftime(b, sizeof b, "%Y-%m-%d %H:%M:%S", localtime(&t));
    return b;
}

}  // namespace hist

class History {
public:
    static constexpr size_t kMaxBytes = 2 * 1024 * 1024;   // これを超えたら history.1.jsonl に回す（1 世代だけ残す）
    static constexpr size_t kMaxInMemory = 20000;

    explicit History(std::string dir) : dir_(std::move(dir)) {}

    std::string path() const { return dir_ + "/history.jsonl"; }
    std::string old_path() const { return dir_ + "/history.1.jsonl"; }

    void load() {
        events_.clear();
        keys_.clear();
        load_file(old_path());
        load_file(path());
        version_++;
    }

    // 追加できたら true（同じ鍵がもうあれば false）。
    // 同じ鍵でも、前の記録にワールドが無くて今回はあるなら、ワールドを書き足して true
    bool add(HistEvent e) {
        if (e.kind != "join" && e.kind != "leave" && e.kind != "world") return false;
        if (e.t.empty()) e.t = hist::now_local();
        if (e.key.empty()) e.key = e.t + "|" + e.kind + "|" + e.name;
        if (keys_.count(e.key)) {
            HistEvent* old = find(e.key);
            if (!old || !fills_in(*old, e)) return false;
            merge(*old, e);
            append(*old);   // 読み直したときは後の行が勝つ
            version_++;
            return true;
        }
        append(e);
        insert_sorted(std::move(e));
        version_++;
        return true;
    }

    const std::deque<HistEvent>& events() const { return events_; }   // 古い順
    unsigned version() const { return version_; }

private:
    // b が a に無いワールドの情報を持っているか
    static bool fills_in(const HistEvent& a, const HistEvent& b) {
        return (a.world.empty() && !b.world.empty()) || (a.wid.empty() && !b.wid.empty() && (a.world.empty() || a.world == b.world));
    }
    static void merge(HistEvent& a, const HistEvent& b) {
        if (a.world.empty()) a.world = b.world;
        if (a.wid.empty() && a.world == b.world) a.wid = b.wid;
    }
    HistEvent* find(const std::string& key) {
        for (auto it = events_.rbegin(); it != events_.rend(); ++it)
            if (it->key == key) return &*it;
        return nullptr;
    }
    void append(const HistEvent& e) {
        mkdirs();
        rotate_if_needed();
        std::ofstream f(path(), std::ios::app);
        if (f) f << hist::to_json(e) << "\n";
    }

    void load_file(const std::string& p) {
        std::ifstream f(p);
        std::string line;
        HistEvent e;
        while (std::getline(f, line)) {
            if (!hist::from_json(line, e)) continue;
            if (!keys_.count(e.key)) {
                insert_sorted(e);
            } else if (HistEvent* old = find(e.key); old && fills_in(*old, e)) {
                merge(*old, e);
            }
        }
    }

    void insert_sorted(HistEvent e) {
        keys_.insert(e.key);
        // ほとんどは末尾。過去の分（Windows 側の読み込み直し）は時刻の位置に入れる
        auto it = events_.end();
        while (it != events_.begin() && (it - 1)->t > e.t) --it;
        events_.insert(it, std::move(e));
        while (events_.size() > kMaxInMemory) events_.pop_front();
    }

    void mkdirs() {
        std::string acc;
        for (size_t i = 0; i <= dir_.size(); i++) {
            if (i == dir_.size() || dir_[i] == '/') {
                if (!acc.empty()) mkdir(acc.c_str(), 0700);
            }
            if (i < dir_.size()) acc += dir_[i];
        }
    }

    void rotate_if_needed() {
        struct stat st {};
        if (stat(path().c_str(), &st) == 0 && size_t(st.st_size) > kMaxBytes) {
            std::rename(path().c_str(), old_path().c_str());   // 前の世代は置き換わる（容量の上限）
        }
    }

    std::string dir_;
    std::deque<HistEvent> events_;
    std::unordered_set<std::string> keys_;
    unsigned version_ = 0;
};
