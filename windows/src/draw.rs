// SPDX-License-Identifier: MIT
//! 小さな描画部品（tiny-skia で図形、fontdue で文字）。0.5 までの Frame 側（cairo + pango）と同じ寸法・色。
//! 文字の大きさは pango と同じ「pt」で受け取り、96 dpi として px にする。
use std::path::PathBuf;
use std::sync::OnceLock;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);
pub const FRIEND: Rgb = Rgb(1.0, 0.84, 0.25); // フレンドの名前（黄色）
pub const SELF: Rgb = Rgb(0.45, 0.86, 1.0); // 自分の名前（水色）
pub const WHITE: Rgb = Rgb(1.0, 1.0, 1.0);

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

// ---------------------------------------------------------------- フォント
pub struct Fonts {
    regular: Vec<fontdue::Font>, // 先頭から順に、字があるものを使う
    bold: Vec<fontdue::Font>,
}

fn load(path: &str, index: u32) -> Option<fontdue::Font> {
    let bytes = std::fs::read(path).ok()?;
    let settings = fontdue::FontSettings { collection_index: index, scale: 40.0, ..Default::default() };
    fontdue::Font::from_bytes(bytes, settings).ok()
}

fn windir() -> PathBuf {
    PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts")
}

/// Windows に入っているフォント（游ゴシック → メイリオ）と記号・絵文字（Segoe UI Symbol / Emoji）。
/// Windows 以外（テスト）では Noto Sans CJK など
fn candidates(bold: bool) -> Vec<(String, u32)> {
    let w = windir();
    let p = |f: &str| w.join(f).to_string_lossy().to_string();
    let mut v = if bold {
        vec![(p("YuGothB.ttc"), 0), (p("meiryob.ttc"), 0), (p("msgothic.ttc"), 0)]
    } else {
        vec![(p("YuGothM.ttc"), 0), (p("YuGothR.ttc"), 0), (p("meiryo.ttc"), 0), (p("msgothic.ttc"), 0)]
    };
    v.push((p("seguisym.ttf"), 0));
    v.push((p("seguiemj.ttf"), 0));
    let (n, d) = if bold { ("Bold", "Bold") } else { ("Regular", "Regular") };
    for dir in ["/usr/share/fonts/opentype/noto", "/usr/share/fonts/noto-cjk", "/usr/share/fonts/google-noto-cjk"] {
        // Noto Sans CJK の ttc は JP が 0 番
        v.push((format!("{dir}/NotoSansCJK-{n}.ttc"), 0));
    }
    v.push((format!("/usr/share/fonts/truetype/dejavu/DejaVuSans{}.ttf", if d == "Bold" { "-Bold" } else { "" }), 0));
    v.push(("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(), 0));
    v
}

pub fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let pick = |bold| candidates(bold).into_iter().filter_map(|(p, i)| load(&p, i)).collect::<Vec<_>>();
        Fonts { regular: pick(false), bold: pick(true) }
    })
}

impl Fonts {
    pub fn available(&self) -> bool {
        !self.regular.is_empty()
    }
    fn list(&self, bold: bool) -> &[fontdue::Font] {
        if bold && !self.bold.is_empty() { &self.bold } else { &self.regular }
    }
    fn font_for(&self, c: char, bold: bool) -> Option<&fontdue::Font> {
        let l = self.list(bold);
        l.iter().find(|f| f.lookup_glyph_index(c) != 0).or_else(|| l.first())
    }
}

fn px(pt: f32) -> f32 {
    pt * 96.0 / 72.0
}

/// 文字の幅（px）
pub fn measure(s: &str, bold: bool, pt: f32) -> f32 {
    let f = fonts();
    let size = px(pt);
    s.chars()
        .filter(|c| !matches!(c, '\u{fe0f}' | '\u{200d}'))
        .map(|c| f.font_for(c, bold).map_or(0.0, |font| font.metrics(c, size).advance_width))
        .sum()
}

// ---------------------------------------------------------------- キャンバス
pub struct Canvas {
    pub pm: Pixmap,
}

fn color(c: Rgb, a: f32) -> Color {
    Color::from_rgba(c.0.clamp(0.0, 1.0), c.1.clamp(0.0, 1.0), c.2.clamp(0.0, 1.0), a.clamp(0.0, 1.0)).unwrap()
}

fn rounded_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w.min(h) / 2.0).max(0.0);
    let k = 0.552_284_8 * r;
    let mut p = PathBuilder::new();
    p.move_to(x + r, y);
    p.line_to(x + w - r, y);
    p.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    p.line_to(x + w, y + h - r);
    p.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    p.line_to(x + r, y + h);
    p.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    p.line_to(x, y + r);
    p.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    p.close();
    p.finish()
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        Canvas { pm: Pixmap::new(w, h).unwrap() }
    }
    pub fn w(&self) -> f32 {
        self.pm.width() as f32
    }
    pub fn h(&self) -> f32 {
        self.pm.height() as f32
    }
    fn paint(c: Rgb, a: f32) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color(color(c, a));
        p.anti_alias = true;
        p
    }
    pub fn clear(&mut self, c: Rgb) {
        self.pm.fill(color(c, 1.0));
    }
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Rgb, a: f32) {
        if let Some(r) = Rect::from_xywh(x, y, w, h) {
            self.pm.fill_rect(r, &Self::paint(c, a), Transform::identity(), None);
        }
    }
    pub fn rounded(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgb, a: f32) {
        if let Some(p) = rounded_path(x, y, w, h, r) {
            self.pm.fill_path(&p, &Self::paint(c, a), FillRule::Winding, Transform::identity(), None);
        }
    }
    pub fn rounded_stroke(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: Rgb, a: f32) {
        if let Some(p) = rounded_path(x, y, w, h, r) {
            let s = tiny_skia::Stroke { width, ..Default::default() };
            self.pm.stroke_path(&p, &Self::paint(c, a), &s, Transform::identity(), None);
        }
    }
    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgb, a: f32) {
        if let Some(p) = PathBuilder::from_circle(cx, cy, r) {
            self.pm.fill_path(&p, &Self::paint(c, a), FillRule::Winding, Transform::identity(), None);
        }
    }

    /// 文字を描く。y は文字の縦の中心。width > 0 なら、はみ出す分を「…」で省略して、その幅の中で揃える。
    /// clip_top / clip_bottom の外には描かない。戻り値は描いた幅（px）
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, s: &str, bold: bool, pt: f32, x: f32, y: f32, width: f32, c: Rgb, align: Align) -> f32 {
        self.text_clip(s, bold, pt, x, y, width, c, align, 0.0, self.h())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn text_clip(&mut self, s: &str, bold: bool, pt: f32, x: f32, y: f32, width: f32, c: Rgb, align: Align, top: f32, bottom: f32) -> f32 {
        let f = fonts();
        let size = px(pt);
        let mut s: String = s.chars().filter(|c| !matches!(c, '\u{fe0f}' | '\u{200d}' | '\n' | '\r')).collect();
        let mut w = measure(&s, bold, pt);
        if width > 0.0 && w > width {
            let mut v: Vec<char> = s.chars().collect();
            while !v.is_empty() && measure(&(v.iter().collect::<String>() + "…"), bold, pt) > width {
                v.pop();
            }
            s = v.iter().collect::<String>() + "…";
            w = measure(&s, bold, pt);
        }
        let x0 = match (align, width > 0.0) {
            (Align::Left, _) => x,
            (Align::Center, true) => x + (width - w) / 2.0,
            (Align::Right, true) => x + width - w,
            (Align::Center, false) => x - w / 2.0,
            (Align::Right, false) => x - w,
        };
        // 縦: 主なフォントの行の高さの真ん中を y に
        let Some(main) = f.list(bold).first() else { return w };
        let lm = main.horizontal_line_metrics(size).unwrap_or(fontdue::LineMetrics {
            ascent: size * 0.88, descent: -size * 0.12, line_gap: 0.0, new_line_size: size,
        });
        let baseline = y + (lm.ascent + lm.descent) / 2.0;
        let (pw, ph) = (self.pm.width() as i32, self.pm.height() as i32);
        let data = self.pm.data_mut();
        let mut pen = x0;
        for ch in s.chars() {
            let Some(font) = f.font_for(ch, bold) else { continue };
            let (m, bmp) = font.rasterize(ch, size);
            let gx = (pen + m.xmin as f32).round() as i32;
            let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
            for row in 0..m.height as i32 {
                let yy = gy + row;
                if yy < 0 || yy >= ph || (yy as f32) < top || (yy as f32) >= bottom {
                    continue;
                }
                for col in 0..m.width as i32 {
                    let xx = gx + col;
                    if xx < 0 || xx >= pw {
                        continue;
                    }
                    let cov = bmp[(row * m.width as i32 + col) as usize] as f32 / 255.0;
                    if cov <= 0.0 {
                        continue;
                    }
                    let i = ((yy * pw + xx) * 4) as usize;
                    // 乗算済みアルファで重ねる
                    let src = [c.0 * cov, c.1 * cov, c.2 * cov, cov];
                    for k in 0..4 {
                        let d = data[i + k] as f32 / 255.0;
                        data[i + k] = ((src[k] + d * (1.0 - cov)) * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            pen += m.advance_width;
        }
        w
    }

    /// OpenVR の SetOverlayRaw に渡す、乗算していない RGBA
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pm.data().len());
        for p in self.pm.pixels() {
            let c = p.demultiply();
            out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        out
    }

    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        self.pm.save_png(path).map_err(|e| e.to_string())
    }
}
