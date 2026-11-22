//! 文字の柄。文字と書式（`TextSpec`）から、柄の絵（RGBA）を描く。
//!
//! フォントは、OS に入っているものを使う（fontique で探し、skrifa で読み、vello_cpu で描く。どれも egui が使っている部品）。
//! 案件にはフォントのファイルを入れない（フォントの使用許諾で、配れないものが多いため）。開いた PC に同じフォントがなければ、
//! 別のフォントで描き、そう知らせる。字形の組み合わせ（カーニング、合字）はしない。1 文字ずつ、送り幅で並べる。

use std::collections::HashMap;
use std::sync::Arc;

use fontique::{Collection, CollectionOptions, SourceCache};
use pakenoba_core::project::{TextAlign, TextSpec};
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};
use vello_cpu::{Glyph, Pixmap, RenderContext, Resources, color, peniko};

/// 文字を描くときの細かさ（1 mm あたりの画素）。約 300 dpi。
const TEXT_PX_PER_MM: f32 = 12.0;
/// 1 枚の絵の画素の上限（大きすぎる文字は、細かさを落として描く）。
const MAX_PIXELS: f32 = 24_000_000.0;
/// 求めたフォントにない文字を探す、ほかのフォント（ある順に使う）。
const FALLBACK: [&str; 12] = ["Yu Gothic UI", "Yu Gothic", "Meiryo", "Hiragino Sans", "Hiragino Kaku Gothic ProN", "Noto Sans CJK JP", "Noto Sans JP", "Source Han Sans JP", "Noto Sans", "DejaVu Sans", "Arial", "Liberation Sans"];

/// 描いた文字の絵。
pub struct TextArt {
    pub spec: TextSpec,
    /// 実際に使ったフォント（求めたフォントがなかったときは、別のもの）。
    pub font_used: String,
    pub w: usize,
    pub h: usize,
    /// RGBA（アルファは乗算なし）。
    pub rgba: Arc<Vec<u8>>,
    /// 柄としての大きさ（mm）。
    pub size_mm: [f32; 2],
}

/// 読み込んだフォント 1 つ。
#[derive(Clone)]
struct Face {
    data: peniko::FontData,
    bytes: Arc<Vec<u8>>,
    index: u32,
}

impl Face {
    fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.bytes, self.index).ok()
    }
}

/// OS のフォント。はじめて使うときに探す（起動を遅くしないため）。
#[derive(Default)]
pub struct Fonts {
    found: Option<(Collection, SourceCache)>,
    names: Vec<String>,
    faces: HashMap<String, Option<Face>>,
}

impl Fonts {
    fn found(&mut self) -> &mut (Collection, SourceCache) {
        self.found.get_or_insert_with(|| (Collection::new(CollectionOptions { shared: false, system_fonts: true }), SourceCache::default()))
    }

    /// 入っているフォントの名前（名前順）。
    pub fn families(&mut self) -> &[String] {
        if self.names.is_empty() {
            let (collection, _) = self.found();
            let mut names: Vec<String> = collection.family_names().map(str::to_owned).collect();
            names.sort_by_key(|n| n.to_lowercase());
            names.dedup();
            self.names = names;
        }
        &self.names
    }

    /// その名前のフォントを読む（ふつうの太さ、立体）。なければ None。
    fn face(&mut self, name: &str) -> Option<Face> {
        if let Some(face) = self.faces.get(name) {
            return face.clone();
        }
        let (collection, cache) = self.found();
        let face = collection.family_by_name(name).and_then(|family| {
            let info = family.match_font(fontique::FontWidth::NORMAL, fontique::FontStyle::Normal, fontique::FontWeight::NORMAL, true).or_else(|| family.default_font())?.clone();
            let blob = info.load(Some(cache))?;
            let bytes = Arc::new(blob.data().to_vec());
            let data = peniko::FontData::new(peniko::Blob::new(bytes.clone()), info.index());
            Some(Face { data, bytes, index: info.index() })
        });
        self.faces.insert(name.to_owned(), face.clone());
        face
    }

    /// 求めたフォントがなかったときに使う、はじめのフォントの名前。
    pub fn default_family(&mut self) -> Option<String> {
        FALLBACK.iter().map(|s| s.to_string()).chain(self.families().first().cloned()).find(|name| self.face(name).is_some())
    }

    /// 文字を描く。
    pub fn render(&mut self, spec: &TextSpec) -> Result<TextArt, String> {
        // 使うフォント: 求めたもの、なければ代わり。足りない文字は、ほかのフォントから探す。
        let mut chain: Vec<(String, Face)> = Vec::new();
        for name in std::iter::once(spec.font.clone()).chain(FALLBACK.iter().map(|s| s.to_string())) {
            if chain.iter().all(|(n, _)| *n != name)
                && let Some(face) = self.face(&name)
            {
                chain.push((name, face));
            }
        }
        if chain.is_empty()
            && let Some(name) = self.families().first().cloned()
            && let Some(face) = self.face(&name)
        {
            chain.push((name, face));
        }
        let Some((first_name, first)) = chain.first().cloned() else { return Err("使えるフォントが見つかりません".to_owned()) };
        let font_used = if self.face(&spec.font).is_some() { spec.font.clone() } else { first_name };

        let lines: Vec<&str> = spec.text.split('\n').collect();
        let area = |px_per_mm: f32| {
            let chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as f32;
            (chars * spec.size * px_per_mm) * (lines.len() as f32 * spec.size * spec.leading.max(1.0) * px_per_mm)
        };
        let px_per_mm = if area(TEXT_PX_PER_MM) > MAX_PIXELS { TEXT_PX_PER_MM * (MAX_PIXELS / area(TEXT_PX_PER_MM)).sqrt() } else { TEXT_PX_PER_MM };
        let em = (spec.size * px_per_mm).max(1.0);
        let first_font = first.font().ok_or("フォントを読めませんでした")?;
        let metrics = first_font.metrics(Size::new(em), LocationRef::default());
        let (ascent, descent) = (metrics.ascent.max(em * 0.5), (-metrics.descent).max(0.0));
        let step = em * spec.leading;
        let spacing = em * spec.spacing / 1000.0;

        // 1 行ずつ、文字を並べる（どのフォントの、どの字形か、横の位置）。
        let mut placed: Vec<Vec<(usize, u32, f32)>> = Vec::new();
        let mut widths = Vec::new();
        for line in &lines {
            let mut x = 0.0f32;
            let mut row = Vec::new();
            for ch in line.chars() {
                let found = chain.iter().enumerate().find_map(|(i, (_, face))| {
                    let font = face.font()?;
                    let gid = font.charmap().map(ch)?;
                    let advance = font.glyph_metrics(Size::new(em), LocationRef::default()).advance_width(gid).unwrap_or(em * 0.5);
                    Some((i, gid.to_u32(), advance))
                });
                let (i, gid, advance) = found.unwrap_or((0, 0, em * 0.5));
                row.push((i, gid, x));
                x += advance + spacing;
            }
            widths.push(if line.is_empty() { 0.0 } else { (x - spacing).max(0.0) });
            placed.push(row);
        }
        let pad = (em * 0.08).ceil();
        let text_w = widths.iter().copied().fold(0.0f32, f32::max).max(em * 0.5);
        let text_h = ascent + descent + step * (lines.len() as f32 - 1.0);
        let (w, h) = ((text_w + pad * 2.0).ceil() as usize, (text_h + pad * 2.0).ceil() as usize);
        if w == 0 || h == 0 || w > u16::MAX as usize || h > u16::MAX as usize {
            return Err("文字が大きすぎます".to_owned());
        }

        let mut ctx = RenderContext::new(w as u16, h as u16);
        let mut resources = Resources::new();
        ctx.set_paint(color::OpaqueColor::<color::Srgb>::from_rgb8(spec.color[0], spec.color[1], spec.color[2]));
        for (font_index, (_, face)) in chain.iter().enumerate() {
            let glyphs: Vec<Glyph> = placed
                .iter()
                .enumerate()
                .flat_map(|(row, glyphs)| {
                    let shift = match spec.align {
                        TextAlign::Left => 0.0,
                        TextAlign::Center => (text_w - widths[row]) * 0.5,
                        TextAlign::Right => text_w - widths[row],
                    };
                    let baseline = pad + ascent + step * row as f32;
                    glyphs.iter().filter(move |g| g.0 == font_index).map(move |&(_, id, x)| Glyph { id, x: pad + shift + x, y: baseline })
                })
                .collect();
            if !glyphs.is_empty() {
                ctx.glyph_run(&mut resources, &face.data).font_size(em).hint(false).fill_glyphs(glyphs.into_iter());
            }
        }
        let mut pixmap = Pixmap::new(w as u16, h as u16);
        ctx.render(&mut pixmap, &mut resources);
        // 乗算済みを、乗算なしに直す。
        let rgba: Vec<u8> = pixmap
            .data_as_u8_slice()
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3];
                if a == 0 {
                    [0, 0, 0, 0]
                } else {
                    let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                    [un(p[0]), un(p[1]), un(p[2]), a]
                }
            })
            .collect();
        Ok(TextArt { spec: spec.clone(), font_used, w, h, rgba: Arc::new(rgba), size_mm: [w as f32 / px_per_mm, h as f32 / px_per_mm] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(text: &str) -> TextSpec {
        TextSpec { text: text.to_owned(), font: "ないフォント".to_owned(), size: 10.0, color: [200, 30, 40], align: TextAlign::Left, spacing: 0.0, leading: 1.5 }
    }

    #[test]
    fn 文字を描くと_色が乗り_行が増えると高くなる() {
        let mut fonts = Fonts::default();
        if fonts.families().is_empty() {
            eprintln!("フォントがない環境なので、確かめない");
            return;
        }
        let one = fonts.render(&spec("箱 Box")).unwrap();
        assert_ne!(one.font_used, "ないフォント", "ないフォントは、別のフォントで描く");
        let ink: Vec<&[u8]> = one.rgba.chunks_exact(4).filter(|p| p[3] > 200).collect();
        assert!(ink.len() > 100, "文字が描かれていない");
        assert!(ink.iter().all(|p| p[0] > 150 && p[1] < 90), "色が違う: {:?}", ink[0]);
        // 大きさは、文字の大きさにだいたい比例する（mm）。
        assert!(one.size_mm[1] > 10.0 && one.size_mm[1] < 18.0, "{:?}", one.size_mm);
        let two = fonts.render(&spec("箱 Box\n二行目")).unwrap();
        assert!((two.size_mm[1] - one.size_mm[1] - 15.0).abs() < 1.0, "行送り 1.5 倍ぶん高くなる: {:?} {:?}", one.size_mm, two.size_mm);
        // 字間を広げると、幅が増える。
        let wide = fonts.render(&TextSpec { spacing: 500.0, ..spec("箱 Box") }).unwrap();
        assert!(wide.size_mm[0] > one.size_mm[0] + 10.0);
        // 右揃えでは、短い行が右に寄る。
        let right = fonts.render(&TextSpec { align: TextAlign::Right, ..spec("長い一行目です\n短") }).unwrap();
        let col = |art: &TextArt, x: usize| (0..art.h).any(|y| art.rgba[(y * art.w + x) * 4 + 3] > 128);
        let lower_left = (right.h * 2 / 3..right.h).any(|y| (0..right.w / 3).any(|x| right.rgba[(y * right.w + x) * 4 + 3] > 128));
        assert!(!lower_left && col(&right, right.w - right.w / 10), "右揃えになっていない");
    }
}
