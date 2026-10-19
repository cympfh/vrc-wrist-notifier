// SPDX-License-Identifier: MIT
//! オーバーレイに絵を渡すときの判断（OS に依らないので Linux でもテストできる）。実際に渡すのは vr.rs / d3d.rs。

/// 前に渡した中身と同じなら渡さない（作り直したオーバーレイには last を空にして渡しなおす）
pub fn should_upload(last: &[u8], new: &[u8]) -> bool {
    last.is_empty() || last != new
}

/// SetOverlayTexture が何回続けて失敗したら、D3D11 をやめて SetOverlayRaw にするか
pub const MAX_TEXTURE_FAILURES: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// うまくいった
    Ok,
    /// この 1 回だけ SetOverlayRaw で渡す（次はまた D3D11 で試す）
    RawThisTime,
    /// D3D11 をやめて、これからはずっと SetOverlayRaw
    GiveUp,
}

/// D3D11 の道が使えるかの記録
#[derive(Debug, Default)]
pub struct TexturePath {
    failures: u32,
}

impl TexturePath {
    /// ok: SetOverlayTexture が 0 を返したか。device_removed: GetDeviceRemovedReason がエラーを返したか
    pub fn record(&mut self, ok: bool, device_removed: bool) -> Next {
        if device_removed {
            return Next::GiveUp;
        }
        if ok {
            self.failures = 0;
            return Next::Ok;
        }
        self.failures += 1;
        if self.failures >= MAX_TEXTURE_FAILURES { Next::GiveUp } else { Next::RawThisTime }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_only_when_unchanged() {
        assert!(should_upload(&[], &[1, 2]));
        assert!(should_upload(&[1, 2], &[1, 3]));
        assert!(!should_upload(&[1, 2], &[1, 2]));
    }

    #[test]
    fn failures_fall_back_then_give_up() {
        let mut p = TexturePath::default();
        assert_eq!(p.record(false, false), Next::RawThisTime);
        assert_eq!(p.record(true, false), Next::Ok); // 成功すると数えなおし
        assert_eq!(p.record(false, false), Next::RawThisTime);
        assert_eq!(p.record(false, false), Next::RawThisTime);
        assert_eq!(p.record(false, false), Next::GiveUp);
        assert_eq!(TexturePath::default().record(true, true), Next::GiveUp); // デバイスが失われたらすぐ
    }
}
