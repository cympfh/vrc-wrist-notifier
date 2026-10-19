// SPDX-License-Identifier: MIT
//! VRChat のキーを %LOCALAPPDATA%\vrc-wrist-notifier\auth.bin に保存する（Windows だけ）。
//! DPAPI（CryptProtectData）で、この PC のこの Windows ユーザーだけが戻せる形に暗号化する。平文では書かない。
//! キーは表示・ログに出さない。使い終わった平文のバッファは 0 で埋める。
use std::path::PathBuf;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

/// アプリごとの追加の鍵（同じユーザーのほかのアプリの DPAPI の結果と取り違えないため。秘密ではない）
const ENTROPY: &[u8] = b"cympfh.vrc-wrist-notifier.auth";

pub fn path() -> PathBuf {
    vwn::config::app_dir().join("auth.bin")
}

pub fn exists() -> bool {
    path().is_file()
}

fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        unsafe { std::ptr::write_volatile(x, 0) };
    }
}

fn blob(b: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: b.len() as u32, pbData: b.as_ptr() as *mut u8 }
}

/// DPAPI の結果を Vec に写して、DPAPI のバッファは 0 で埋めて返す
unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let s = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
    let v = s.to_vec();
    wipe(s);
    LocalFree(out.pbData as _);
    v
}

fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    let (inp, ent) = (blob(plain), blob(ENTROPY));
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let ok = unsafe {
        CryptProtectData(&inp, std::ptr::null(), &ent, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
    };
    if ok == 0 {
        return Err(format!("暗号化できませんでした（{}）", std::io::Error::last_os_error()));
    }
    Ok(unsafe { take(out) })
}

fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
    let (inp, ent) = (blob(sealed), blob(ENTROPY));
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let ok = unsafe {
        CryptUnprotectData(
            &inp, std::ptr::null_mut(), &ent, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out,
        )
    };
    (ok != 0).then(|| unsafe { take(out) })
}

/// 保存したキー（無い・戻せないときは None）
pub fn load() -> Option<String> {
    let file = std::fs::read(path()).ok()?;
    let mut plain = unprotect(vwn::auth::unwrap(&file)?)?;
    let key = std::str::from_utf8(&plain).ok().map(String::from);
    wipe(&mut plain);
    key.filter(|k| !k.is_empty())
}

/// 暗号化して保存する（一時ファイルに書いてから置き換える）
pub fn save(key: &str) -> Result<(), String> {
    let data = vwn::auth::wrap(&protect(key.as_bytes())?);
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("bin.tmp");
    std::fs::write(&tmp, &data).map_err(|e| format!("{} に書けません: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("{} に置き換えられません: {e}", p.display()))
}

/// 設定の窓の「消す」: 保存したキーを消す
pub fn clear() -> Result<(), String> {
    match std::fs::remove_file(path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
