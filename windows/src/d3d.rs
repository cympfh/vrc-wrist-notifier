// SPDX-License-Identifier: MIT
//! オーバーレイに絵を渡すための D3D11 のテクスチャ（Windows だけ）。
//! SetOverlayRaw は渡すたびに SteamVR の中でテクスチャを作り直すので、見えているオーバーレイに渡すと一瞬透明になる（ちらつく）。
//! そこで、オーバーレイごとに作りっぱなしのテクスチャを 2 枚持ち、交互に中身を書き換えて SetOverlayTexture で渡す
//! （SteamVR がまだ前の 1 枚を読んでいても、書き換えるのはもう 1 枚のほう）。
use std::ffi::c_void;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
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
            D3D11CreateDevice(ad.as_ref(), driver, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION,
                              Some(&mut device), None, Some(&mut ctx))
                .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
            Ok(Gpu { device: device.ok_or("D3D11 のデバイスがありません")?, ctx: ctx.ok_or("D3D11 のコンテキストがありません")? })
        }
    }

    fn texture(&self, w: u32, h: u32) -> Result<ID3D11Texture2D, String> {
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
        t.ok_or_else(|| "CreateTexture2D".into())
    }

    pub fn surface(&self, w: u32, h: u32) -> Result<Surface, String> {
        Ok(Surface { tex: [self.texture(w, h)?, self.texture(w, h)?], next: 0, w, h, last: Vec::new() })
    }
}

/// 1 つのオーバーレイ用の 2 枚のテクスチャ（作ったら最後まで同じものを使う。大きさも変えない）
pub struct Surface {
    tex: [ID3D11Texture2D; 2],
    next: usize,
    pub w: u32,
    pub h: u32,
    /// 最後に渡した中身（同じなら渡さない）
    last: Vec<u8>,
}

impl Surface {
    /// rgba（w×h、乗算していない RGBA）を次の 1 枚に書き、そのテクスチャのポインタを返す。前と同じ中身なら None
    pub fn write(&mut self, gpu: &Gpu, rgba: &[u8]) -> Option<*mut c_void> {
        if rgba.len() != (self.w * self.h * 4) as usize || rgba == self.last.as_slice() {
            return None;
        }
        let t = &self.tex[self.next];
        unsafe {
            gpu.ctx.UpdateSubresource(t, 0, None, rgba.as_ptr() as *const c_void, self.w * 4, 0);
            gpu.ctx.Flush(); // SteamVR が読む前に書き終わっているように
        }
        self.next ^= 1;
        self.last.clear();
        self.last.extend_from_slice(rgba);
        Some(t.as_raw())
    }
    pub fn forget_last(&mut self) {
        self.last.clear();
    }
}
