// SPDX-License-Identifier: MIT
//! VRChat のキー（`auth` クッキーの値）まわりの、OS に依らない部分。
//! 入力の整え方、保存するファイルの形、問題があるときの一言、設定の窓に出す今の状態。
//! 暗号化（DPAPI）とファイルの読み書きは Windows だけの keystore.rs。キーは表示・ログに出さない。
//! friends の状態: ok / none / off / nokey（未入力）/ bad（401 など、キーが効かない）/ unreadable（auth.bin を戻せない）/ neterror（オフライン・5xx など）

/// 保存するファイル（auth.bin）の先頭。その後ろは DPAPI で暗号化したキー
pub const MAGIC: &[u8] = b"VWNK1\n";

/// トレイのメニュー・ツールチップに出す一言（問題があるときだけ）
pub const TRAY_NOKEY: &str = "⚠ VRChat のキーが未入力です（「設定…」から入力）";
pub const TRAY_BAD: &str = "⚠ VRChat のキーが使えません（「設定…」から入れなおす）";
/// SteamVR のパネルの右下に出す一言（頭に ⚠ が付く）
pub const PANEL_NOKEY: &str = "VRChat のキーが未入力です（PC のトレイの「設定…」から）";
pub const PANEL_BAD: &str = "VRChat のキーが使えません（PC のトレイの「設定…」から）";
/// 保存したキー（auth.bin）を戻せない（別の PC・別のユーザーからコピーした、壊れた、など）
pub const UNREADABLE: &str = "保存したキーを読めません（入れなおしてください）";
/// VRChat に問い合わせられない（オフライン・5xx など）。キーのせいではないので、入れなおしは勧めない
pub const TRAY_NET: &str = "⚠ VRChat に問い合わせられません（あとでまた試します）";

#[derive(Debug, PartialEq, Eq)]
pub enum KeyError {
    Empty,
    Invalid,
}

impl KeyError {
    pub fn message(&self) -> &'static str {
        match self {
            KeyError::Empty => "キーを貼り付けてください",
            KeyError::Invalid => "キーの形ではありません（authcookie_ で始まる値を貼り付けてください）",
        }
    }
}

/// 貼り付けられたものからキーの値だけを取り出す。
/// 前後の空白・引用符、`auth=…`、`Cookie: auth=…; …` の形も受け付ける
pub fn normalize_key(input: &str) -> Result<String, KeyError> {
    let mut s = input.trim();
    // 先頭 7 バイトが文字の切れ目でないとき（日本語など）は get が None になるので、落ちない
    if s.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("cookie:")) {
        s = s[7..].trim();
    }
    // 「名前=値; 名前=値」の形なら auth の値だけ
    if s.contains('=') {
        let found = s.split(';').map(str::trim).find_map(|kv| kv.strip_prefix("auth="));
        match found {
            Some(v) => s = v.trim(),
            None => return Err(KeyError::Invalid),
        }
    }
    let s = s.trim_matches(|c| c == '"' || c == '\'').trim();
    if s.is_empty() {
        return Err(KeyError::Empty);
    }
    let ok = s.len() <= 1024 && s.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b';' | b',' | b'"' | b'\\' | b'='));
    if !ok {
        return Err(KeyError::Invalid);
    }
    Ok(s.to_string())
}

/// 暗号化したものを auth.bin の中身にする
pub fn wrap(sealed: &[u8]) -> Vec<u8> {
    let mut v = MAGIC.to_vec();
    v.extend_from_slice(sealed);
    v
}

/// auth.bin の中身から暗号化したものを取り出す（形が違えば None）
pub fn unwrap(file: &[u8]) -> Option<&[u8]> {
    file.strip_prefix(MAGIC).filter(|b| !b.is_empty())
}

/// トレイに出す一言（friends は Status.friends: ok / nokey / bad / none / off）
pub fn tray_problem(friends: &str) -> Option<String> {
    match friends {
        "nokey" => Some(TRAY_NOKEY.into()),
        "bad" => Some(TRAY_BAD.into()),
        "unreadable" => Some(format!("⚠ {UNREADABLE}")),
        "neterror" => Some(TRAY_NET.into()),
        _ => None,
    }
}

/// パネルの右下に出す一言（問い合わせられないのは一時的で、VR の中では何もできないので出さない）
pub fn panel_problem(friends: &str) -> Option<&'static str> {
    match friends {
        "nokey" => Some(PANEL_NOKEY),
        "bad" => Some(PANEL_BAD),
        "unreadable" => Some(UNREADABLE),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Good,
    Warn,
}

/// 設定の窓の「今の状態」の 1 行
pub fn settings_status(saved: bool, friends: &str, refreshing: bool, count: usize) -> (String, Tone) {
    if !saved {
        return ("まだ入力されていません".into(), Tone::Neutral);
    }
    if refreshing {
        return ("保存済み。使えるか確かめています…".into(), Tone::Neutral);
    }
    match friends {
        "ok" => (format!("✓ 保存済み。使えています（フレンド {count} 人）"), Tone::Good),
        "bad" => ("⚠ 保存済みですが使えません。新しいキーを入れなおしてください".into(), Tone::Warn),
        "unreadable" => (format!("⚠ {UNREADABLE}"), Tone::Warn),
        "neterror" => ("保存済み。VRChat に問い合わせられませんでした（あとでまた試します）".into(), Tone::Neutral),
        "off" => ("保存済み（config.json でフレンドの判定がオフです）".into(), Tone::Neutral),
        _ => ("保存済み".into(), Tone::Neutral),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_common_pastes() {
        let k = "authcookie_0123abcd-ef45-6789-abcd-0123456789ab";
        for input in [
            k.to_string(),
            format!("  {k}\r\n"),
            format!("\"{k}\""),
            format!("auth={k}"),
            format!("Cookie: auth={k}; twoFactorAuth=xyz"),
            format!("twoFactorAuth=xyz; auth={k};"),
        ] {
            assert_eq!(normalize_key(&input).as_deref(), Ok(k), "{input:?}");
        }
    }

    #[test]
    fn normalize_rejects_bad_input() {
        assert_eq!(normalize_key(""), Err(KeyError::Empty));
        assert_eq!(normalize_key("   \n"), Err(KeyError::Empty));
        assert_eq!(normalize_key("\"\""), Err(KeyError::Empty));
        assert_eq!(normalize_key("auth="), Err(KeyError::Empty));
        assert_eq!(normalize_key("has space"), Err(KeyError::Invalid));
        assert_eq!(normalize_key("twoFactorAuth=xyz"), Err(KeyError::Invalid));
        assert_eq!(normalize_key("キー"), Err(KeyError::Invalid));
        assert_eq!(normalize_key(&"a".repeat(1025)), Err(KeyError::Invalid));
        // 先頭 7 バイトが文字の切れ目でないもの（以前はここで落ちていた）
        assert_eq!(normalize_key("認証キー"), Err(KeyError::Invalid));
        assert_eq!(normalize_key("認証キー authcookie_xxxx"), Err(KeyError::Invalid));
        assert_eq!(normalize_key("abcdefé"), Err(KeyError::Invalid));
        assert_eq!(normalize_key("Cookie:認証"), Err(KeyError::Invalid));
        // ヘッダーを足そうとするもの
        let k = "authcookie_0123abcd";
        assert_eq!(normalize_key(&format!("{k}\r\nX-Evil: 1")), Err(KeyError::Invalid));
        assert_eq!(normalize_key(&format!("{k}\nX")), Err(KeyError::Invalid));
        assert!(normalize_key(&"a".repeat(1024)).is_ok());
    }

    #[test]
    fn wrap_roundtrip() {
        let b = wrap(&[1, 2, 3]);
        assert!(b.starts_with(MAGIC));
        assert_eq!(unwrap(&b), Some(&[1u8, 2, 3][..]));
        assert_eq!(unwrap(MAGIC), None);
        assert_eq!(unwrap(b"authcookie_plain"), None);
        assert_eq!(unwrap(b""), None);
    }

    #[test]
    fn problems_only_when_needed() {
        for f in ["ok", "none", "off", ""] {
            assert_eq!(tray_problem(f), None);
            assert_eq!(panel_problem(f), None);
        }
        assert!(tray_problem("nokey").unwrap().contains("設定…"));
        assert!(tray_problem("bad").unwrap().contains("使えません"));
        assert!(panel_problem("nokey").unwrap().contains("トレイ"));
        assert!(!panel_problem("bad").unwrap().starts_with('⚠'));
        // 読めないときは窓とトレイとパネルで同じ文言
        assert!(tray_problem("unreadable").unwrap().ends_with(UNREADABLE));
        assert_eq!(panel_problem("unreadable"), Some(UNREADABLE));
        assert!(settings_status(true, "unreadable", false, 0).0.ends_with(UNREADABLE));
        // 問い合わせられないときは、入れなおしを勧めない。パネルには出さない
        assert!(!tray_problem("neterror").unwrap().contains("入れなおし"));
        assert!(!settings_status(true, "neterror", false, 0).0.contains("入れなおし"));
        assert_eq!(panel_problem("neterror"), None);
    }

    #[test]
    fn settings_status_lines() {
        assert_eq!(settings_status(false, "ok", false, 3).1, Tone::Neutral);
        assert_eq!(settings_status(false, "nokey", true, 0).0, "まだ入力されていません");
        assert!(settings_status(true, "ok", true, 3).0.contains("確かめて"));
        let (s, t) = settings_status(true, "ok", false, 249);
        assert!(s.contains("249") && t == Tone::Good);
        assert_eq!(settings_status(true, "bad", false, 0).1, Tone::Warn);
        assert_eq!(settings_status(true, "none", false, 0).0, "保存済み");
    }
}
