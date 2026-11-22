//! 展開図の絵柄とガイド線、PSD との行き来。
//!
//! 絵柄は展開図まるごと 1 枚の画像として持つ（左上が展開図の左上）。

use craft_psd::{BlendMode, LayerSpec, PixelData, PsdBuilder, PsdFile};

use pakenoba_core::template::{P2, Role, Template, inside};

pub const PX_PER_MM: f32 = 4.0;
const PAPER: [u8; 3] = [240, 236, 228];

/// 展開図の画像。どちらも RGBA 8bit（アルファは乗算なし）で、同じ大きさ。
pub struct SheetArt {
    pub w: usize,
    pub h: usize,
    pub art: Vec<u8>,
    /// ガイド（切り線、折り線、のりしろの斜線）。紙の表に重ねる。
    pub guides: Vec<u8>,
    /// 切り線と折り線だけ。紙の裏に重ねる（のりを付けるのは表の面なので、裏には斜線を出さない）。
    pub lines: Vec<u8>,
}

fn sheet_size(tpl: &Template) -> (usize, usize) {
    (((tpl.max[0] - tpl.min[0]) * PX_PER_MM).ceil().max(2.0) as usize, ((tpl.max[1] - tpl.min[1]) * PX_PER_MM).ceil().max(2.0) as usize)
}

fn to_px(tpl: &Template, p: P2) -> [f32; 2] {
    [(p[0] - tpl.min[0]) * PX_PER_MM, (tpl.max[1] - p[1]) * PX_PER_MM]
}

/// 紙の色を、行の束 1 つぶん塗る。`band` は `first_row` 行目からの画素。
fn fill_band(tpl: &Template, w: usize, h: usize, first_row: usize, band: &mut [u8]) {
    let last_row = (first_row + band.len() / (w * 4)).min(h);
    for panel in &tpl.panels {
        let px: Vec<[f32; 2]> = panel.outline.iter().map(|&p| to_px(tpl, p)).collect();
        let x0 = px.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let x1 = (px.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min(w);
        let y0 = (px.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize).max(first_row);
        let y1 = (px.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min(last_row);
        let mut crossings: Vec<f32> = Vec::new();
        for y in y0..y1 {
            // この行が輪郭を横切る位置を求めて、その間だけを塗る（画素ごとに内側かどうかを調べるより、ずっと速い）。
            let v = tpl.max[1] - (y as f32 + 0.5) / PX_PER_MM;
            crossings.clear();
            for i in 0..panel.outline.len() {
                let (a, b) = (panel.outline[i], panel.outline[(i + 1) % panel.outline.len()]);
                if (a[1] > v) != (b[1] > v) {
                    crossings.push(a[0] + (v - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            crossings.sort_by(f32::total_cmp);
            let column = |u: f32| (((u - tpl.min[0]) * PX_PER_MM - 0.5).ceil().max(0.0) as usize).clamp(x0, x1);
            for x in crossings.chunks_exact(2).flat_map(|pair| column(pair[0])..column(pair[1])) {
                let u = tpl.min[0] + (x as f32 + 0.5) / PX_PER_MM;
                if tpl.in_hole(u, v) {
                    continue;
                }
                // まだ絵を置いていない紙の色。組み上がると隠れる部分（のりしろ、差し込みなど）は、少し濃くして見分ける。
                let color = match panel.role {
                    Role::Face | Role::Fin => [246.0, 244.0, 238.0],
                    Role::GlueTab => [228.0, 220.0, 204.0],
                    Role::Tuck | Role::Flap | Role::Gusset => [234.0, 230.0, 222.0],
                };
                let o = ((y - first_row) * w + x) * 4;
                band[o..o + 4].copy_from_slice(&[color[0] as u8, color[1] as u8, color[2] as u8, 255]);
            }
        }
    }
}

/// 何も描いていない紙と、ガイド線（切り線、折り線、のりを付ける面）を作る。
/// 線分（両端）。
pub type Segment = (P2, P2);

/// 展開図の切り線と折り線（展開図の mm）。
/// 2 枚のパネルが展開図の上で同じ辺を共有しているところは、紙がつながっているので、切り線ではなく折り線。
pub fn sheet_lines(tpl: &Template) -> (Vec<Segment>, Vec<Segment>) {
    let same = |p: P2, q: P2| (p[0] - q[0]).abs() < 0.05 && (p[1] - q[1]).abs() < 0.05;
    let edges: Vec<Vec<(P2, P2)>> = tpl.panels.iter().map(|panel| (0..panel.outline.len()).map(|i| (panel.outline[i], panel.outline[(i + 1) % panel.outline.len()])).collect()).collect();
    let (mut cuts, mut folds) = (Vec::new(), Vec::new());
    for (index, own) in edges.iter().enumerate() {
        for &(a, b) in own {
            let other = edges.iter().enumerate().any(|(j, theirs)| j != index && theirs.iter().any(|&(c, d)| same(a, d) && same(b, c)));
            if other {
                // 共有している辺は、両方のパネルに 1 回ずつ出てくるので、片方だけ取る。
                if !folds.iter().any(|&(c, d): &(P2, P2)| same(a, d) && same(b, c)) {
                    folds.push((a, b));
                }
            } else {
                cuts.push((a, b));
            }
        }
    }
    (cuts, folds)
}

pub fn make_sheet_art(tpl: &Template) -> SheetArt {
    let (w, h) = sheet_size(tpl);
    // 透明な部分にも紙の色を入れておく（拡大縮小したときに縁が黒ずまないように）。
    let mut art: Vec<u8> = [PAPER[0], PAPER[1], PAPER[2], 0].repeat(w * h);

    // 行の束ごとに、別のスレッドで塗る（寸法のスライダーを動かすたびに作り直すので、速さが効く）。
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    let band_rows = h.div_ceil(threads).max(32);
    std::thread::scope(|scope| {
        for (band_index, band) in art.chunks_mut(band_rows * w * 4).enumerate() {
            scope.spawn(move || fill_band(tpl, w, h, band_index * band_rows, band));
        }
    });

    // ガイド線。切り線（輪郭）を先に描き、折り線を上から重ねる。
    // 2 枚のパネルが展開図の上で同じ辺を共有しているところは、紙がつながっているので、切り線ではなく折り線。
    // （ゲーブル型の側面の上の、斜めの線。データの上では親子になっていないが、1 枚の紙に入った折り線。）
    const CUT: [u8; 4] = [30, 30, 34, 255];
    const FOLD: [u8; 4] = [0, 150, 220, 255];
    let mut lines = vec![0u8; w * h * 4];
    let mut glue = vec![0u8; w * h * 4];
    let (cuts, folds) = sheet_lines(tpl);
    for (a, b) in cuts {
        draw_line(&mut lines, w, h, to_px(tpl, a), to_px(tpl, b), 1.6, CUT);
    }
    for (a, b) in folds {
        draw_line(&mut lines, w, h, to_px(tpl, a), to_px(tpl, b), 1.6, FOLD);
    }
    // のりしろには、のりを付ける面だと分かるよう、斜めの細い線を入れる。
    const GLUE: [u8; 4] = [226, 138, 40, 255];
    for panel in tpl.panels.iter().filter(|p| p.role == Role::GlueTab) {
        let px: Vec<[f32; 2]> = panel.outline.iter().map(|&p| to_px(tpl, p)).collect();
        let x0 = px.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let x1 = (px.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min(w);
        let y0 = px.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let y1 = (px.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let (u, v) = (tpl.min[0] + (x as f32 + 0.5) / PX_PER_MM, tpl.max[1] - (y as f32 + 0.5) / PX_PER_MM);
                if ((u - v) / 3.0).rem_euclid(1.0) < 0.14 && inside(&panel.outline, [u, v]) {
                    glue[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&GLUE);
                }
            }
        }
    }
    // 抜き穴の輪郭も切り線として描く。
    for hole in &tpl.holes {
        let point = |i: usize| {
            let a = std::f32::consts::TAU * i as f32 / 64.0;
            let (c, s) = (a.cos(), a.sin());
            to_px(tpl, [hole[0] + hole[2] * c.signum() * c.abs().sqrt(), hole[1] + hole[3] * s.signum() * s.abs().sqrt()])
        };
        for i in 0..64 {
            draw_line(&mut lines, w, h, point(i), point(i + 1), 1.6, [30, 30, 34, 255]);
        }
    }
    for hinge in tpl.panels.iter().filter_map(|p| p.hinge.as_ref()) {
        draw_line(&mut lines, w, h, to_px(tpl, hinge.a), to_px(tpl, hinge.b), 1.6, [0, 150, 220, 255]);
    }
    // 曲線の折り線。
    for curved in tpl.panels.iter().filter_map(|p| p.curved.as_ref()) {
        for pair in curved.line.windows(2) {
            draw_line(&mut lines, w, h, to_px(tpl, pair[0]), to_px(tpl, pair[1]), 1.6, [0, 150, 220, 255]);
        }
    }
    // 表のガイドは、のりしろの斜線の上に、線を重ねたもの。
    let mut guides = glue;
    alpha_over(&mut guides, &lines);
    SheetArt { w, h, art, guides, lines }
}

fn draw_line(buf: &mut [u8], w: usize, h: usize, a: [f32; 2], b: [f32; 2], width: f32, color: [u8; 4]) {
    let half = width * 0.5;
    let x0 = (a[0].min(b[0]) - half).floor().max(0.0) as usize;
    let x1 = ((a[0].max(b[0]) + half).ceil().max(0.0) as usize).min(w.saturating_sub(1));
    let y0 = (a[1].min(b[1]) - half).floor().max(0.0) as usize;
    let y1 = ((a[1].max(b[1]) + half).ceil().max(0.0) as usize).min(h.saturating_sub(1));
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (x as f32 + 0.5 - a[0], y as f32 + 0.5 - a[1]);
            let s = ((px * dx + py * dy) / len2).clamp(0.0, 1.0);
            let dist = ((px - s * dx).powi(2) + (py - s * dy).powi(2)).sqrt();
            if dist <= half {
                let o = (y * w + x) * 4;
                buf[o..o + 4].copy_from_slice(&color);
            }
        }
    }
}

/// `over` を `base` の上に重ねる（どちらも RGBA、同じ大きさ）。
fn alpha_over(base: &mut [u8], over: &[u8]) {
    for (b, o) in base.chunks_exact_mut(4).zip(over.chunks_exact(4)) {
        let a = o[3] as f32 / 255.0;
        if a <= 0.0 {
            continue;
        }
        for i in 0..3 {
            b[i] = (o[i] as f32 * a + b[i] as f32 * (1.0 - a)).round() as u8;
        }
        b[3] = (255.0 * (a + b[3] as f32 / 255.0 * (1.0 - a))).round() as u8;
    }
}

/// 紙の裏の色。立体の表示で、柄のない内側の面に使う色と同じ。
const BACK_PAPER: [u8; 3] = [231, 226, 215];

/// 紙の裏（箱の内側の面）の画像を作る。紙の地、裏に置いた柄（`decals` で描く）、切り線と折り線。
pub fn compose_back(sheet: &SheetArt, custom: Option<&[u8]>, show_guides: bool, decals: impl FnOnce(&mut [u8])) -> Vec<u8> {
    // 形は表と同じで、色だけが裏の色。裏の絵を読み込んであれば、それを敷く（表の向きに直したもの）。
    let mut out: Vec<u8> = match custom {
        Some(custom) => custom.to_vec(),
        None => back_paper(sheet),
    };
    decals(&mut out);
    if show_guides {
        alpha_over(&mut out, &sheet.lines);
    }
    out
}

/// 箱に貼る画像を作る。`custom` があれば確認用の絵柄の代わりに使う。
/// 立体の上で置いた柄は、絵柄の上、ガイド線の下に `decals` で描く。
pub fn compose(sheet: &SheetArt, custom: Option<&[u8]>, show_guides: bool, decals: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut out = custom.unwrap_or(&sheet.art).to_vec();
    decals(&mut out);
    if show_guides {
        alpha_over(&mut out, &sheet.guides);
    }
    out
}

/// 立体の上で置いた柄のレイヤーの名前は、この文字で始まる。
pub const DECAL_LAYER_PREFIX: &str = "立体の柄";

/// 型紙を PSD にする。レイヤーは下から「絵柄」、立体の上で置いた柄（1 つにつき 1 レイヤー）、「ガイド」。
///
/// `decals` は、柄ごとの名前と、展開図の大きさの透明な画像。合わせ目をまたぐ柄は、1 枚のレイヤーの中で分かれて描かれている。
/// レイヤーは、描かれている範囲だけに切り詰めて入れる。
/// 型紙の PSD に入れる、柄 1 つぶんのレイヤー。画像は展開図の大きさ。
#[derive(Clone)]
pub struct DecalLayer {
    pub name: String,
    pub image: Vec<u8>,
    /// 隠した柄は、隠したレイヤーとして入れる（統合した絵には入れない）。
    pub visible: bool,
}

#[cfg(test)]
impl DecalLayer {
    pub fn shown(name: impl Into<String>, image: Vec<u8>) -> DecalLayer {
        DecalLayer { name: name.into(), image, visible: true }
    }
}

/// 紙の裏の、何も描いていない絵（形は表と同じで、裏の色）。
fn back_paper(sheet: &SheetArt) -> Vec<u8> {
    sheet.art.chunks_exact(4).flat_map(|p| [BACK_PAPER[0], BACK_PAPER[1], BACK_PAPER[2], p[3]]).collect()
}

/// 裏の型紙の、いちばん下のレイヤーの名前。読み込むときに、この名前で裏の型紙だと見分ける。
pub const BACK_PAPER_LAYER: &str = "紙の裏";

/// パケノバが書き出した、裏の型紙の PSD か。
pub fn psd_is_back(bytes: &[u8]) -> bool {
    PsdFile::from_bytes(bytes).is_ok_and(|file| {
        let names: Vec<String> = file.iter_layers().map(|layer| layer.name()).collect();
        names.iter().any(|n| n == BACK_PAPER_LAYER) && names.iter().any(|n| is_generated(n))
    })
}

/// 型紙を PSD にする。`custom` は、読み込んである展開図の絵（あれば「絵柄」のレイヤーにする）。
pub fn save_psd(sheet: &SheetArt, custom: Option<&[u8]>, decals: &[DecalLayer]) -> Result<Vec<u8>, String> {
    build_psd(sheet.w, sheet.h, ("絵柄", custom.map_or_else(|| sheet.art.clone(), <[u8]>::to_vec)), decals.to_vec(), (format!("{GUIDE_LAYER_PREFIX}（切り線、折り線、のりしろ）"), sheet.guides.clone()))
}

/// 紙の裏の型紙を PSD にする。裏から見た向き（表を左右に裏返した向き）で書く。紙を裏返して刷るときの、そのままの向き。
///
/// レイヤーは下から「紙の裏」、裏に置いた柄、「ガイド（切り線、折り線）」。のりしろの斜線は入れない（のりを付けるのは表の面）。
/// `decals` は、表と同じ展開図の向きで描いた、裏の柄の画像（`paint` で裏を描いたもの）。ここで左右を裏返す。
/// `custom` は、読み込んである裏の絵（表の向きに直したもの）。あれば「紙の裏」のレイヤーにする。
pub fn save_back_psd(sheet: &SheetArt, custom: Option<&[u8]>, decals: &[DecalLayer]) -> Result<Vec<u8>, String> {
    let paper: Vec<u8> = custom.map_or_else(|| back_paper(sheet), <[u8]>::to_vec);
    let flip = |image: &[u8]| mirror(image, sheet.w, sheet.h);
    let decals = decals.iter().map(|layer| DecalLayer { image: flip(&layer.image), ..layer.clone() }).collect();
    build_psd(sheet.w, sheet.h, (BACK_PAPER_LAYER, flip(&paper)), decals, (format!("{GUIDE_LAYER_PREFIX}（切り線、折り線）"), flip(&sheet.lines)))
}

/// 画像を左右に裏返す。
pub fn mirror(image: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; image.len()];
    for y in 0..h {
        for x in 0..w {
            let (from, to) = ((y * w + x) * 4, (y * w + (w - 1 - x)) * 4);
            out[to..to + 4].copy_from_slice(&image[from..from + 4]);
        }
    }
    out
}

/// PSD を組み立てる。いちばん下に `base`、その上に柄、いちばん上に `guides`。
fn build_psd(w: usize, h: usize, base: (&str, Vec<u8>), decals: Vec<DecalLayer>, guides: (String, Vec<u8>)) -> Result<Vec<u8>, String> {
    let mut composite: Vec<u8> = [255u8, 255, 255, 255].repeat(w * h);
    alpha_over(&mut composite, &base.1);
    for layer in decals.iter().filter(|layer| layer.visible) {
        alpha_over(&mut composite, &layer.image);
    }
    alpha_over(&mut composite, &guides.1);
    let mut builder = PsdBuilder::new(w as u32, h as u32);
    builder.push_layer(LayerSpec::new(base.0, 0, 0, w as u32, h as u32, PixelData::Rgba8(base.1)));
    for DecalLayer { name, image, visible } in decals {
        // 何か描かれている範囲。
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
        for y in 0..h {
            for x in 0..w {
                if image[(y * w + x) * 4 + 3] > 0 {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                }
            }
        }
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let cropped: Vec<u8> = (y0..y1).flat_map(|y| image[(y * w + x0) * 4..(y * w + x1) * 4].iter().copied()).collect();
        let mut spec = LayerSpec::new(format!("{DECAL_LAYER_PREFIX} {name}"), x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32, PixelData::Rgba8(cropped));
        spec.visible = visible;
        builder.push_layer(spec);
    }
    builder.push_layer(LayerSpec::new(guides.0, 0, 0, w as u32, h as u32, PixelData::Rgba8(guides.1)));
    builder.composite(PixelData::Rgba8(composite));
    builder.to_bytes().map_err(|e| format!("PSD を書けませんでした: {e}"))
}

/// ガイドのレイヤーの名前は、この文字で始まる。読み込むときに、この名前で見分けて除く。
pub const GUIDE_LAYER_PREFIX: &str = "ガイド";

/// このソフトが書き出したレイヤーのうち、読み込むときに除くもの（ガイドと、立体の上で置いた柄）。
/// 柄は、このソフトの中に動かせる形で残っているので、絵として二重に読み込まない。
fn is_generated(name: &str) -> bool {
    name.starts_with(GUIDE_LAYER_PREFIX) || name.starts_with(DECAL_LAYER_PREFIX)
}

/// 読み込んだ PSD に入っていたガイドの扱い。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideHandling {
    /// ガイドのレイヤーはなかった。
    NotPresent,
    /// ガイドのレイヤーを除いて、残りのレイヤーを重ねた。
    Removed,
    /// ガイドのレイヤーはあったが、除けなかった（統合画像をそのまま使った）。
    Baked,
}

/// 読み込んだ PSD。画像は、PSD の大きさのまま（展開図に合わせるのは `fit`）。
pub struct LoadedPsd {
    /// 合わせる前の画像（大きさは `source_size`）。寸法を変えたときに、ここから合わせ直す。
    pub source: Vec<u8>,
    pub source_size: [usize; 2],
    pub layer_names: Vec<String>,
    pub has_real_merged: Option<bool>,
    pub guides: GuideHandling,
}

/// ガイド以外のレイヤーを下から順に重ねる。
///
/// 扱えるのは、通常の重ね方（不透明度つき）の画素レイヤーだけ。グループ、マスク、クリッピング、
/// 通常以外の合成モードがあれば None を返す（本番では PhotoCraft のエンジンに任せる部分）。
fn composite_without_guides(file: &PsdFile, w: usize, h: usize) -> Option<Vec<u8>> {
    let mut canvas: Vec<u8> = [PAPER[0], PAPER[1], PAPER[2], 0].repeat(w * h);
    for layer in file.iter_layers() {
        let record = layer.record;
        if is_generated(&layer.name()) || !record.is_visible() {
            continue;
        }
        if record.blend_mode != BlendMode::Normal || record.clipping != 0 || record.section_divider().is_some() || layer.user_mask().is_some() {
            return None;
        }
        let image = layer.rgba8().ok()?;
        let (lw, lh) = (image.width as usize, image.height as usize);
        if image.data.len() != lw * lh * 4 {
            return None;
        }
        let opacity = record.opacity as f32 / 255.0 * record.fill_opacity() as f32 / 255.0;
        for ly in 0..lh {
            let y = image.top as i64 + ly as i64;
            if y < 0 || y >= h as i64 {
                continue;
            }
            for lx in 0..lw {
                let x = image.left as i64 + lx as i64;
                if x < 0 || x >= w as i64 {
                    continue;
                }
                let src = &image.data[(ly * lw + lx) * 4..(ly * lw + lx) * 4 + 4];
                let a = src[3] as f32 / 255.0 * opacity;
                if a <= 0.0 {
                    continue;
                }
                let o = (y as usize * w + x as usize) * 4;
                let below = canvas[o + 3] as f32 / 255.0;
                let out_a = a + below * (1.0 - a);
                for i in 0..3 {
                    canvas[o + i] = ((src[i] as f32 * a + canvas[o + i] as f32 * below * (1.0 - a)) / out_a).round() as u8;
                }
                canvas[o + 3] = (out_a * 255.0).round() as u8;
            }
        }
    }
    Some(canvas)
}

/// PSD を読み、展開図の大きさ（`w` x `h`）に合わせる。
///
/// ガイドのレイヤーが入っていれば、それを除いた絵にする。外部アプリでそのまま書き出すと、
/// 統合画像にはガイドの線が焼き込まれているため。
pub fn load_psd(bytes: &[u8]) -> Result<LoadedPsd, String> {
    let file = PsdFile::from_bytes(bytes).map_err(|e| format!("PSD を読めませんでした: {e}"))?;
    let image = file.composite_rgba8().map_err(|e| format!("統合画像を取り出せませんでした: {e}"))?;
    let (sw, sh) = (image.width as usize, image.height as usize);
    if sw == 0 || sh == 0 || image.data.len() != sw * sh * 4 {
        return Err(format!("統合画像の大きさが合いません（{sw}x{sh}）"));
    }
    let layer_names: Vec<String> = file.iter_layers().map(|l| l.name()).collect();
    let has_guides = layer_names.iter().any(|n| is_generated(n));
    let (source, guides) = match (has_guides, has_guides.then(|| composite_without_guides(&file, sw, sh)).flatten()) {
        (false, _) => (image.data, GuideHandling::NotPresent),
        (true, Some(clean)) => (clean, GuideHandling::Removed),
        (true, None) => (image.data, GuideHandling::Baked),
    };
    Ok(LoadedPsd { source, source_size: [sw, sh], layer_names, has_real_merged: file.has_real_merged_data(), guides })
}

/// 柄の素材にする PSD の、統合した絵（RGBA）と大きさ。
pub fn psd_picture(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let file = PsdFile::from_bytes(bytes).map_err(|e| format!("PSD を読めませんでした: {e}"))?;
    let image = file.composite_rgba8().map_err(|e| format!("PSD の統合画像を取り出せませんでした: {e}"))?;
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 || image.data.len() != w * h * 4 {
        return Err(format!("PSD の統合画像の大きさが合いません（{w}x{h}）"));
    }
    Ok((w, h, image.data))
}

/// パケノバが書き出した型紙の PSD か（ガイドのレイヤーがあるか）。そうなら、柄の素材ではなく、展開図まるごとの絵として読む。
pub fn psd_is_dieline(bytes: &[u8]) -> bool {
    PsdFile::from_bytes(bytes).is_ok_and(|file| file.iter_layers().any(|layer| is_generated(&layer.name())))
}

/// テスト用の小さな PSD。`dieline` が真なら、パケノバの型紙と同じガイドのレイヤーを持つ。
#[cfg(test)]
pub fn tests_psd(w: usize, h: usize, dieline: bool) -> Vec<u8> {
    let art = [30u8, 120, 200, 255].repeat(w * h);
    let guide_name = if dieline { format!("{GUIDE_LAYER_PREFIX}（切り線）") } else { "飾り".to_owned() };
    build_psd(w, h, ("絵", art), Vec::new(), (guide_name, vec![0u8; w * h * 4])).unwrap()
}

/// 画像を、展開図の大きさ（`w` x `h`）に合わせる。近い画素を拾うだけの単純な拡大縮小。
pub fn fit(source: &[u8], [sw, sh]: [usize; 2], w: usize, h: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let sy = (y * sh / h).min(sh - 1);
        for x in 0..w {
            let sx = (x * sw / w).min(sw - 1);
            rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&source[(sy * sw + sx) * 4..(sy * sw + sx) * 4 + 4]);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dieline_psd_round_trips() {
        let tpl = Template::tuck_box(120.0, 60.0, 160.0, 1.5);
        let sheet = make_sheet_art(&tpl);
        assert_eq!(sheet.art.len(), sheet.w * sheet.h * 4);
        assert_eq!(sheet.guides.len(), sheet.w * sheet.h * 4);
        // 絵柄にもガイドにも、何かが描かれている。
        assert!(sheet.art.chunks_exact(4).any(|p| p[3] == 255));
        assert!(sheet.guides.chunks_exact(4).any(|p| p[3] == 255));

        let bytes = save_psd(&sheet, None, &[]).unwrap();
        let loaded = load_psd(&bytes).unwrap();
        let fitted = fit(&loaded.source, loaded.source_size, sheet.w, sheet.h);
        assert_eq!(loaded.source_size, [sheet.w, sheet.h]);
        assert_eq!(loaded.layer_names, vec!["絵柄".to_owned(), "ガイド（切り線、折り線、のりしろ）".to_owned()]);
        assert_eq!(fitted.len(), sheet.w * sheet.h * 4);
        // 貼る画像は、絵柄とガイドを重ねたものと同じ大きさになる。
        assert_eq!(compose(&sheet, Some(&fitted), true, |_| {}).len(), sheet.art.len());
    }

    #[test]
    fn decal_layer_is_saved_and_left_out_on_load() {
        use crate::paint::{Canvas, Marks, Picture, Placed, paint};
        use pakenoba_core::surface::{Decal, Surface};
        let tpl = Template::tuck_box(120.0, 60.0, 160.0, 1.5);
        let surface = Surface::new(&tpl, &Template::tuck_box(120.0, 60.0, 160.0, 0.02));
        let sheet = make_sheet_art(&tpl);
        let mut layer = vec![0u8; sheet.w * sheet.h * 4];
        let logo = Placed { picture: Picture::Logo, decal: Decal { panel: 0, at: [0.5, 0.5], size: [40.0, 40.0], angle: 0.0 }, back: false, layer: Default::default() };
        paint(&surface, &tpl, &[logo], Marks::default(), false, Canvas { w: sheet.w, h: sheet.h, rgba: &mut layer });
        let painted: Vec<usize> = (0..sheet.w * sheet.h).filter(|&i| layer[i * 4 + 3] == 255).collect();
        assert!(painted.len() > 10_000);
        let mut band_layer = vec![0u8; sheet.w * sheet.h * 4];
        let band = Placed { picture: Picture::Band, decal: Decal { panel: 0, at: [0.5, 0.2], size: [360.0, 14.0], angle: 0.0 }, back: false, layer: Default::default() };
        paint(&surface, &tpl, &[band], Marks::default(), false, Canvas { w: sheet.w, h: sheet.h, rgba: &mut band_layer });

        // 柄 1 つにつき 1 レイヤー。何も描かれていない柄は入れない。
        // 隠した柄（帯）は、隠したレイヤーとして入れる。
        let band_pixels: Vec<usize> = (0..sheet.w * sheet.h).filter(|&i| band_layer[i * 4 + 3] == 255 && layer[i * 4 + 3] == 0 && sheet.guides[i * 4 + 3] == 0).collect();
        let decals = [DecalLayer { name: "1（帯）".to_owned(), image: band_layer, visible: false }, DecalLayer::shown("2（ロゴ）", layer.clone()), DecalLayer::shown("3（空）", vec![0u8; sheet.w * sheet.h * 4])];
        let bytes = save_psd(&sheet, None, &decals).unwrap();
        let file = PsdFile::from_bytes(&bytes).unwrap();
        let hidden: Vec<String> = file.iter_layers().filter(|l| !l.record.is_visible()).map(|l| l.name()).collect();
        assert_eq!(hidden, vec!["立体の柄 1（帯）".to_owned()]);
        let composite = file.composite_rgba8().unwrap();
        for &i in band_pixels.iter().step_by(53) {
            assert_eq!(&composite.data[i * 4..i * 4 + 3], &sheet.art[i * 4..i * 4 + 3], "隠した柄が、統合した絵に入っている");
        }
        // ロゴのレイヤーは、描かれている範囲だけに切り詰められて、元の位置に入っている。
        let file = PsdFile::from_bytes(&bytes).unwrap();
        let saved = file.iter_layers().find(|l| l.name() == "立体の柄 2（ロゴ）").unwrap().rgba8().unwrap();
        let (left, top, width) = (saved.left as usize, saved.top as usize, saved.width as usize);
        assert!(width < 200 && (saved.height as usize) < 200, "切り詰められていない: {width} x {}", saved.height);
        for &i in painted.iter().step_by(131) {
            let (x, y) = (i % sheet.w, i / sheet.w);
            let o = ((y - top) * width + (x - left)) * 4;
            assert_eq!(&saved.data[o..o + 4], &layer[i * 4..i * 4 + 4]);
        }
        let loaded = load_psd(&bytes).unwrap();
        let fitted = fit(&loaded.source, loaded.source_size, sheet.w, sheet.h);
        assert_eq!(loaded.layer_names, vec!["絵柄".to_owned(), "立体の柄 1（帯）".to_owned(), "立体の柄 2（ロゴ）".to_owned(), "ガイド（切り線、折り線、のりしろ）".to_owned()]);
        assert_eq!(loaded.guides, GuideHandling::Removed);
        // 柄のあった画素でも、読み込んだ絵は元の絵柄のまま（柄が絵として焼き込まれていない）。
        for &i in painted.iter().step_by(97) {
            assert_eq!(&fitted[i * 4..i * 4 + 4], &sheet.art[i * 4..i * 4 + 4]);
        }
    }

    #[test]
    fn art_fills_exactly_the_panels() {
        // 行ごとの塗りが、画素ごとに内側かどうかを調べた結果と一致する。
        for tpl in [Template::tuck_box(120.0, 60.0, 160.0, 1.5), Template::pillow(180.0, 110.0, 0.5, 1.2), Template::gable(160.0, 100.0, 110.0, 45.0, 0.5)] {
            let sheet = make_sheet_art(&tpl);
            for y in (0..sheet.h).step_by(3) {
                for x in (0..sheet.w).step_by(3) {
                    let at = [tpl.min[0] + (x as f32 + 0.5) / PX_PER_MM, tpl.max[1] - (y as f32 + 0.5) / PX_PER_MM];
                    let expected = tpl.panels.iter().any(|p| inside(&p.outline, at)) && !tpl.in_hole(at[0], at[1]);
                    assert_eq!(sheet.art[(y * sheet.w + x) * 4 + 3] == 255, expected, "{at:?}");
                }
            }
        }
    }

    #[test]
    fn shared_edges_are_drawn_as_folds() {
        // ゲーブル型の側面の上の斜めの線は、2 枚の三角形が共有する辺。切り線ではなく折り線の色で描かれる。
        let tpl = Template::gable(160.0, 100.0, 110.0, 45.0, 0.5);
        let sheet = make_sheet_art(&tpl);
        let center = tpl.panels.iter().find(|p| p.name == "右の三角").unwrap();
        // 底辺の左端から頂点へ向かう斜めの線の、中ほどの点。
        let (base, apex) = (center.outline.iter().copied().min_by(|a, b| (a[0] + a[1]).total_cmp(&(b[0] + b[1]))).unwrap(), center.outline.iter().copied().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap());
        let mid = to_px(&tpl, [(base[0] + apex[0]) * 0.5, (base[1] + apex[1]) * 0.5]);
        let o = (mid[1] as usize * sheet.w + mid[0] as usize) * 4;
        assert_eq!(&sheet.guides[o..o + 4], &[0, 150, 220, 255]);
    }

    #[test]
    fn 裏の型紙は_左右を裏返して書く() {
        let tpl = Template::tuck_box(120.0, 60.0, 160.0, 0.5);
        let sheet = make_sheet_art(&tpl);
        // 左上の隅に、印の画素を 1 つだけ描いた柄。
        let mut mark = vec![0u8; sheet.w * sheet.h * 4];
        let (x, y) = (sheet.w / 5, sheet.h / 2);
        mark[(y * sheet.w + x) * 4..(y * sheet.w + x) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
        let bytes = save_back_psd(&sheet, None, &[DecalLayer::shown("1（ロゴ）", mark)]).unwrap();
        let file = PsdFile::from_bytes(&bytes).unwrap();
        let names: Vec<String> = file.iter_layers().map(|l| l.name()).collect();
        assert_eq!(names, ["紙の裏", "立体の柄 1（ロゴ）", "ガイド（切り線、折り線）"]);
        // 印は、左右が入れ替わった場所にある。
        let image = file.composite_rgba8().unwrap();
        let at = |x: usize| &image.data[(y * sheet.w + x) * 4..(y * sheet.w + x) * 4 + 4];
        assert_eq!(at(sheet.w - 1 - x), &[255, 0, 0, 255]);
        assert_ne!(at(x), &[255, 0, 0, 255]);
        // 読み込むときは、ガイドと柄のレイヤーを除く（表と同じ決まり）。
        assert_eq!(load_psd(&bytes).unwrap().guides, GuideHandling::Removed);
        // 左右を 2 回裏返すと、元に戻る。
        assert_eq!(mirror(&mirror(&sheet.lines, sheet.w, sheet.h), sheet.w, sheet.h), sheet.lines);
    }

    #[test]
    fn guides_are_removed_on_load() {
        let tpl = Template::tuck_box(120.0, 60.0, 160.0, 1.5);
        let sheet = make_sheet_art(&tpl);
        let loaded = load_psd(&save_psd(&sheet, None, &[]).unwrap()).unwrap();
        let fitted = fit(&loaded.source, loaded.source_size, sheet.w, sheet.h);
        assert_eq!(loaded.guides, GuideHandling::Removed);
        // ガイドの線があった画素でも、読み込んだ絵は元の絵柄と同じになる（線が焼き込まれていない）。
        let mut on_guides = 0;
        for i in 0..sheet.w * sheet.h {
            if sheet.guides[i * 4 + 3] == 255 && sheet.art[i * 4 + 3] == 255 {
                assert_eq!(&fitted[i * 4..i * 4 + 4], &sheet.art[i * 4..i * 4 + 4]);
                on_guides += 1;
            }
        }
        assert!(on_guides > 1000);
    }
}
