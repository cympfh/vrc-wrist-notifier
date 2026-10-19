// SPDX-License-Identifier: MIT
//! オーバーレイに絵を渡すための D3D11 のテクスチャ（Windows だけ）。
//! SetOverlayRaw で見えているオーバーレイに渡すと一瞬透明になる（ちらつく）のが見えたので、オーバーレイごとに作りっぱなしの
//! テクスチャを 1 枚持ち、中身を書き換えて SetOverlayTexture で渡す。SteamVR はこちらのデバイスの即時コンテキストに
//! コピーを積むので、SetOverlayTexture の後で Flush する（openvr #1484 / #1353）。コピーされるので 1 枚で足りる。
use std::ffi::c_void;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_FLAG,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1};

pub struct Gpu {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
}

impl Gpu {
    /// adapter: SteamVR が使っている GPU の番号（IVRSystem::GetDXGIOutputInfo）。見つからなければ既定の GPU
    pub fn new(adapter: Option<u32>) -> Result<Gpu, String> {
        unsafe {
            let ad: Option<IDXGIAdapter> = adapter.and_then(|i| {
                let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
                f.EnumAdapters1(i).ok()?.cast().ok()
            });
            let driver = if ad.is_some() { D3D_DRIVER_TYPE_UNKNOWN } else { D3D_DRIVER_TYPE_HARDWARE };
            let (mut device, mut ctx) = (None, None);
            // RGBA のテクスチャを書き換えるだけなので、BGRA 対応などのフラグは要らない
            D3D11CreateDevice(ad.as_ref(), driver, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), None, D3D11_SDK_VERSION,
                              Some(&mut device), None, Some(&mut ctx))
                .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
            Ok(Gpu { device: device.ok_or("D3D11 のデバイスがありません")?, ctx: ctx.ok_or("D3D11 のコンテキストがありません")? })
        }
    }

    pub fn surface(&self, w: u32, h: u32) -> Result<Surface, String> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as _,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut t = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut t)) }.map_err(|e| format!("CreateTexture2D: {e}"))?;
        Ok(Surface { tex: t.ok_or("CreateTexture2D")?, w, h })
    }

    /// SetOverlayTexture の後に呼ぶ（SteamVR が積んだコピーをすぐ流す）
    pub fn flush(&self) {
        unsafe { self.ctx.Flush() };
    }

    /// デバイスが失われていれば true（GPU のリセット・ドライバの更新など）
    pub fn removed(&self) -> bool {
        unsafe { self.device.GetDeviceRemovedReason() }.is_err()
    }
}

/// 1 つのオーバーレイ用のテクスチャ（作ったら最後まで同じものを使う。大きさも変えない）
pub struct Surface {
    tex: ID3D11Texture2D,
    w: u32,
    h: u32,
}

impl Surface {
    /// rgba（w×h、乗算していない RGBA）で中身を書き換え、テクスチャのポインタを返す
    pub fn write(&self, gpu: &Gpu, rgba: &[u8], w: u32, h: u32) -> Result<*mut c_void, String> {
        if (w, h) != (self.w, self.h) || rgba.len() != (w * h * 4) as usize {
            return Err(format!("大きさが違います（{w}×{h}、テクスチャは {}×{}）", self.w, self.h));
        }
        unsafe { gpu.ctx.UpdateSubresource(&self.tex, 0, None, rgba.as_ptr() as *const c_void, w * 4, 0) };
        Ok(self.tex.as_raw())
    }
}
