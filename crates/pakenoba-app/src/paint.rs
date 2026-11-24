//! 柄の絵と、展開図の画像への描き込み。
//!
//! 柄の場所と、隣の面への続き方は、中核（`pakenoba_core::surface`）が決める。ここは、絵を画素にするところ。
//! いまは展開図の画像に描き込んでいる。あとで、柄ごとに元の解像度のまま描く形に置き換える。

use pakenoba_i18n::t;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use pakenoba_core::surface::{Decal, Surface};
use pakenoba_core::template::{P2, Template, inside};

use crate::art::{BleedZone, Frame, PX_PER_MM};
use pakenoba_core::project::Blend;

/// 柄に重ねて描く印。選んでいる柄には点線の枠、カーソルの下の柄には色の枠。
#[derive(Clone, Copy, Default)]
pub struct Marks<'a> {
    pub selected: &'a [usize],
    /// カーソルの下の柄（どの表示でも、同じ柄を光らせる）。
    pub hover: Option<usize>,
}

/// カーソルの下の柄の枠の色（パケノバの色）。
const HOVER: [f32; 4] = [38.0, 178.0, 170.0, 1.0];

/// 立体の上に置いた柄。場所と大きさ（`Decal`）に、絵を添えたもの。
#[derive(Clone)]
pub struct Placed {
    pub decal: Decal,
    pub picture: Picture,
    /// 紙の裏（箱の内側の面）に置いた柄か。
    /// 場所は、表の柄と同じく展開図の座標で持つ。絵は、裏から見て正しく読めるように、左右を逆にして描く。
    pub back: bool,
    /// レイヤーとしての名前、表示、ロック。
    pub layer: pakenoba_core::project::Layer,
}

impl Deref for Placed {
    type Target = Decal;
    fn deref(&self) -> &Decal {
        &self.decal
    }
}

impl DerefMut for Placed {
    fn deref_mut(&mut self) -> &mut Decal {
        &mut self.decal
    }
}

/// 表か裏の、どちらかの面にある柄のうち、つかめるもの（隠していない、ロックしていない）の場所を取り出す（中核の関数に渡すため）。
/// 返すのは、もとの番号の並びと、場所の並び。
pub fn spots_on(decals: &[Placed], back: bool) -> (Vec<usize>, Vec<Decal>) {
    decals.iter().enumerate().filter(|(i, p)| p.back == back && is_artwork(&p.picture) && !p.layer.fixed() && shown_in_group(decals, *i) && !group_of(decals, *i).is_some_and(|g| decals[g].layer.fixed())).map(|(index, p)| (index, p.decal)).unzip()
}

/// 柄の絵。
#[derive(Clone)]
pub enum Picture {
    /// 丸いロゴ（確認用。上向きの矢印で、向きが分かる）。
    Logo,
    /// 帯（確認用。右向きの山形の並びで、向きと続き方が分かる）。
    Band,
    /// 読み込んだ画像（RGBA、アルファは乗算なし）。`vector` なら、SVG を細かく描いた絵（素材は SVG のまま）。
    Image { w: usize, h: usize, rgba: Arc<Vec<u8>>, vector: bool },
    /// 文字（文字と書式から描いた絵）。
    Text(Arc<crate::text::TextArt>),
    /// 図形（形と色から、その場で描く。拡大してもぼやけない）。
    Shape(pakenoba_core::project::ShapeSpec),
    /// グループの見出し（絵はない）。`open` はレイヤーの欄で開いているか。
    Group { open: bool },
    /// 調整レイヤー（絵はない。下の絵に、レイヤーの調整を効かせる）。
    Adjustment,
}

impl Picture {
    pub fn label(&self) -> &'static str {
        match self {
            Picture::Logo => t!("Logo"),
            Picture::Band => t!("Band"),
            Picture::Image { vector: true, .. } => t!("Vector"),
            Picture::Image { .. } => t!("Image"),
            Picture::Text(_) => t!("Text"),
            Picture::Shape(_) => t!("Shape"),
            Picture::Group { .. } => t!("Group"),
            Picture::Adjustment => t!("Adjustment layer"),
        }
    }

    /// 柄の座標 `q`（中心が原点、mm、上が正）での色。RGB は 0〜255、アルファは 0〜1。縁は画面の細かさでなだらかにする。
    pub fn sample(&self, q: P2, size: P2) -> [f32; 4] {
        self.sample_at(q, size, PX_PER_MM)
    }

    /// `sample` と同じ。縁は 1 mm あたり `ppm` 画素の細かさでなだらかにする（書き出し用）。
    pub fn sample_at(&self, q: P2, size: P2, ppm: f32) -> [f32; 4] {
        const TEAL: [f32; 3] = [8.0, 126.0, 139.0];
        const AMBER: [f32; 3] = [242.0, 169.0, 0.0];
        const DARK: [f32; 3] = [23.0, 54.0, 56.0];
        const WHITE: [f32; 3] = [255.0, 252.0, 245.0];
        let half = [size[0] * 0.5, size[1] * 0.5];
        match self {
            Picture::Logo => {
                let radius = half[0].min(half[1]);
                let (x, y) = (q[0] / radius, q[1] / radius);
                let rho = (x * x + y * y).sqrt();
                // 縁は 1 画素ぶんの幅でなだらかにする。
                let alpha = ((1.0 - rho) * radius * ppm + 0.5).clamp(0.0, 1.0);
                let head = (0.05..=0.60).contains(&y) && x.abs() < (0.60 - y) * 0.95;
                let stem = (-0.55..=0.05).contains(&y) && x.abs() < 0.17;
                let rgb = if rho > 0.84 {
                    AMBER
                } else if head || stem {
                    WHITE
                } else {
                    TEAL
                };
                [rgb[0], rgb[1], rgb[2], alpha]
            }
            Picture::Band => {
                let y = q[1].abs() / half[1];
                let period = size[1] * 1.5;
                let phase = (q[0] + half[0] + q[1].abs() * 0.9).rem_euclid(period);
                let rgb = if y > 0.78 || (y < 0.60 && phase < period * 0.32) { DARK } else { AMBER };
                [rgb[0], rgb[1], rgb[2], 1.0]
            }
            Picture::Text(art) => Picture::sample_rgba(art.w, art.h, &art.rgba, q, size),
            Picture::Shape(spec) => Picture::sample_shape(spec, q, half, ppm),
            Picture::Image { w, h, rgba, .. } => Picture::sample_rgba(*w, *h, rgba, q, size),
            Picture::Group { .. } | Picture::Adjustment => [0.0; 4],
        }
    }

    /// 図形の、柄の座標 `q` での色。縁からの距離（内側が負）で、塗りと線を決める。縁は 1 画素ぶんでなだらかにする。
    fn sample_shape(spec: &pakenoba_core::project::ShapeSpec, q: P2, half: P2, ppm: f32) -> [f32; 4] {
        use pakenoba_core::project::ShapeKind;
        let d = match spec.kind {
            ShapeKind::Rect => {
                let r = spec.radius.min(half[0]).min(half[1]).max(0.0);
                let (dx, dy) = (q[0].abs() - (half[0] - r), q[1].abs() - (half[1] - r));
                (dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)) - r
            }
            ShapeKind::Ellipse => {
                // 楕円までの距離の近似（中心からの正規化した距離で、短いほうの半径に合わせる）。
                let n = (q[0] / half[0].max(1e-3)).hypot(q[1] / half[1].max(1e-3));
                (n - 1.0) * half[0].min(half[1])
            }
        };
        // 縁は、外へ半画素だけなめらかにする（中心が形の内側の画素は、すべて塗る）。隣り合うパネルに、面の大きさちょうどの
        // 図形を置いたとき、折り線の上の画素が両方から半分ずつ塗られて薄くなり、明るい線が出るのを防ぐため。
        let cover = |edge: f32| (1.0 - edge * ppm).clamp(0.0, 1.0);
        let inside = cover(d);
        if inside <= 0.0 {
            return [0.0; 4];
        }
        let fill = spec.fill.map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]);
        let stroke = spec.stroke.map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]);
        // 線は、縁から内側へ太さのぶん。太さのぶん縮めた形の外側が線。
        let on_line = if stroke.is_some() { 1.0 - cover(d + spec.stroke_width) } else { 0.0 };
        match (fill, stroke) {
            (Some(f), Some(s)) => {
                let t = on_line;
                [f[0] + (s[0] - f[0]) * t, f[1] + (s[1] - f[1]) * t, f[2] + (s[2] - f[2]) * t, inside]
            }
            (Some(f), None) => [f[0], f[1], f[2], inside],
            (None, Some(s)) => [s[0], s[1], s[2], inside * on_line],
            (None, None) => [0.0; 4],
        }
    }

    /// RGBA の画像（アルファは乗算なし）の、柄の座標 `q` での色。画素の中心どうしの間を、なめらかにつなぐ。
    fn sample_rgba(w: usize, h: usize, rgba: &[u8], q: P2, size: P2) -> [f32; 4] {
        let half = [size[0] * 0.5, size[1] * 0.5];
        let fx = ((q[0] + half[0]) / size[0] * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
        let fy = ((half[1] - q[1]) / size[1] * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let texel = |x: usize, y: usize| {
            let o = (y * w + x) * 4;
            let a = rgba[o + 3] as f32 / 255.0;
            [rgba[o] as f32 * a, rgba[o + 1] as f32 * a, rgba[o + 2] as f32 * a, a]
        };
        let (p00, p10, p01, p11) = (texel(x0, y0), texel(x1, y0), texel(x0, y1), texel(x1, y1));
        let mut out = [0.0f32; 4];
        for i in 0..4 {
            out[i] = (p00[i] * (1.0 - tx) + p10[i] * tx) * (1.0 - ty) + (p01[i] * (1.0 - tx) + p11[i] * tx) * ty;
        }
        if out[3] <= 0.0 { [0.0; 4] } else { [out[0] / out[3], out[1] / out[3], out[2] / out[3], out[3]] }
    }
}

/// 描き込む先の、展開図の画像（RGBA、左上が展開図の左上）。
pub struct Canvas<'a> {
    pub w: usize,
    pub h: usize,
    pub rgba: &'a mut [u8],
    /// 調整した絵ができるまで待つ（書き出すとき）。待たないときは、できるまで元の絵で描く。
    pub wait: bool,
    /// 画素と mm の対応。None なら画面の細かさ（展開図の範囲ちょうど、1 mm 4 画素）。
    pub frame: Option<Frame>,
    /// 塗り足し。あれば、柄をパネルの面のまま、切り線の外へ伸ばして描く。
    pub zone: Option<&'a BleedZone>,
}

impl<'a> Canvas<'a> {
    /// 画面に出すための描き込み先（調整した絵を待たない）。
    pub fn screen(w: usize, h: usize, rgba: &'a mut [u8]) -> Canvas<'a> {
        Canvas { w, h, rgba, wait: false, frame: None, zone: None }
    }

    /// 書き出すための描き込み先（調整した絵ができるまで待つ）。
    pub fn output(w: usize, h: usize, rgba: &'a mut [u8]) -> Canvas<'a> {
        Canvas { w, h, rgba, wait: true, frame: None, zone: None }
    }

    /// 書き出すための描き込み先で、細かさと余白を決めたもの。
    pub fn at(frame: Frame, rgba: &'a mut [u8]) -> Canvas<'a> {
        Canvas { w: frame.w, h: frame.h, rgba, wait: true, frame: Some(frame), zone: None }
    }

    /// `at` と同じ。塗り足しまで描く。
    pub fn bled(frame: Frame, zone: Option<&'a BleedZone>, rgba: &'a mut [u8]) -> Canvas<'a> {
        Canvas { w: frame.w, h: frame.h, rgba, wait: true, frame: Some(frame), zone }
    }
}

/// 柄として、場所と大きさを持つものか（グループの見出しと調整レイヤーは持たない）。
pub fn is_artwork(picture: &Picture) -> bool {
    !matches!(picture, Picture::Group { .. } | Picture::Adjustment)
}

/// レイヤーの、グループの入れ子の深さ。
pub fn depth(placed: &Placed) -> u8 {
    placed.layer.depth
}

/// 柄 `index` が、グループごと隠れていないか（入っているグループのどれかが隠れていれば、中身も見えない）。
pub fn shown_in_group(decals: &[Placed], index: usize) -> bool {
    let Some(placed) = decals.get(index) else { return false };
    !placed.layer.hidden && ancestors(decals, index).iter().all(|&g| !decals[g].layer.hidden)
}

/// 柄 `index` が入っているグループの見出しの番号（いちばん内側）。同じ面で、上にある、より浅い最初のもの。
pub fn group_of(decals: &[Placed], index: usize) -> Option<usize> {
    let placed = decals.get(index)?;
    let d = depth(placed);
    if d == 0 {
        return None;
    }
    let back = placed.back;
    (index + 1..decals.len()).filter(|&i| decals[i].back == back).find(|&i| depth(&decals[i]) < d).filter(|&i| matches!(decals[i].picture, Picture::Group { .. }))
}

/// 柄 `index` が入っているグループの見出しすべて（内側から外側へ）。
pub fn ancestors(decals: &[Placed], index: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = index;
    while let Some(g) = group_of(decals, at) {
        out.push(g);
        at = g;
    }
    out
}

/// グループ `header` の中身すべて（入れ子の中身も。下から）。
pub fn members_of(decals: &[Placed], header: usize) -> Vec<usize> {
    let back = decals[header].back;
    let d = depth(&decals[header]);
    let mut out: Vec<usize> = (0..header).rev().filter(|&i| decals[i].back == back).take_while(|&i| depth(&decals[i]) > d).collect();
    out.reverse();
    out
}

/// 柄 `index` がクリップしている土台（同じ面、同じ深さで、すぐ下のもの。クリップしているものが続けば、その下の土台）。
pub fn clip_base(decals: &[Placed], index: usize) -> Option<usize> {
    let placed = decals.get(index)?;
    if !placed.layer.clipped {
        return None;
    }
    let (back, d) = (placed.back, depth(placed));
    let mut at = index;
    loop {
        let below = (0..at).rev().filter(|&i| decals[i].back == back).find(|&i| depth(&decals[i]) <= d)?;
        if depth(&decals[below]) != d {
            return None;
        }
        if !decals[below].layer.clipped {
            return Some(below);
        }
        at = below;
    }
}

/// 画面に描いた、敷き詰めた柄の絵（柄と並べ方と画面の大きさで引く）。いくつかだけ覚える。
static TILED: std::sync::Mutex<Vec<(String, Arc<Vec<u8>>)>> = std::sync::Mutex::new(Vec::new());

/// 敷き詰めた柄の絵。覚えていればそれを、なければ `make` で描いて覚える。
fn tiled_layer(key: &str, make: impl FnOnce() -> Vec<u8>) -> Arc<Vec<u8>> {
    {
        let mut kept = TILED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = kept.iter().position(|(k, _)| k == key) {
            let hit = kept.remove(at);
            let layer = hit.1.clone();
            kept.push(hit);
            return layer;
        }
    }
    let layer = Arc::new(make());
    let mut kept = TILED.lock().unwrap_or_else(|e| e.into_inner());
    kept.push((key.to_owned(), layer.clone()));
    // 表と裏で数枚ずつ。古いものから忘れる。
    while kept.len() > 4 {
        kept.remove(0);
    }
    layer
}

/// 描くときに使う、決まった値。
struct Painter<'a> {
    surface: &'a Surface,
    tpl: &'a Template,
    decals: &'a [Placed],
    back: bool,
    w: usize,
    frame: Frame,
    wait: bool,
    zone: Option<&'a BleedZone>,
}

impl Painter<'_> {
    /// 柄がかかる画素ごとに、`f(画素の番号, その画素の値（target の per 個）, 柄の座標, 柄の縁からの距離)` を呼ぶ。
    /// 柄の座標は、裏の柄では左右を逆にしたもの（絵の向き）。広い範囲は、行の束に分けて、別のスレッドで描く。
    /// `tiled` なら、タイルの設定（`Layer::tile`）のある柄を、並べて敷き詰めた形で描く。
    fn each_pixel<T: Send>(&self, decal: &Placed, target: &mut [T], per: usize, tiled: bool, f: impl Fn(usize, &mut [T], P2, f32) + Sync) {
        if let Some(tile) = decal.layer.tile.filter(|t| tiled && !t.off) {
            return self.each_tiled_pixel(decal, tile, target, per, f);
        }
        let (surface, tpl, w, back, frame) = (self.surface, self.tpl, self.w, self.back, &self.frame);
        let half = [decal.size[0] * 0.5, decal.size[1] * 0.5];
        // 近いパネルのぶんを最後に描く（一周して重なったところでは、手前のものが上に来る）。
        for piece in surface.layout(tpl, decal).iter().rev() {
            let outline = &tpl.panels[piece.panel].outline;
            let to_decal = piece.place.inverse();
            // 描く範囲は、柄の四隅とパネルの輪郭の、両方に収まる画素だけ。
            let corners = [[-half[0], -half[1]], [half[0], -half[1]], [half[0], half[1]], [-half[0], half[1]]].map(|c| piece.place.point(c));
            let span = |points: &mut dyn Iterator<Item = P2>| points.fold(([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
            let (lo_d, hi_d) = span(&mut corners.iter().copied());
            let (lo_p, hi_p) = span(&mut outline.iter().copied());
            // 塗り足しがあれば、パネルの外へ、その幅まで描く。
            let grow = self.zone.map_or(0.0, |z| z.bleed);
            let (lo_p, hi_p) = ([lo_p[0] - grow, lo_p[1] - grow], [hi_p[0] + grow, hi_p[1] + grow]);
            let (lo, hi) = ([lo_d[0].max(lo_p[0]), lo_d[1].max(lo_p[1])], [hi_d[0].min(hi_p[0]), hi_d[1].min(hi_p[1])]);
            if lo[0] >= hi[0] || lo[1] >= hi[1] {
                continue;
            }
            let (x0, x1, y0, y1) = frame.span(lo, hi);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            // 行 `rows` を、その行の束 `band`（target の y0 行目から）に描く。
            let rows = |first: usize, band: &mut [T]| {
                let count = band.len() / (w * per);
                for y in first..first + count {
                    for x in x0..x1 {
                        let at = frame.at(x, y);
                        let q = to_decal.point(at);
                        if q[0].abs() > half[0] || q[1].abs() > half[1] {
                            continue;
                        }
                        let shown = match self.zone {
                            // 塗り足しのあるとき: 抜き穴の中も描く（抜いて捨てる所なので）。紙の外は、いちばん近いパネルの柄だけ。
                            Some(zone) => inside(outline, at) || zone.reaches(y * w + x, piece.panel, at, |p| {
                                let q = to_decal.point(p);
                                q[0].abs() <= half[0] && q[1].abs() <= half[1]
                            }),
                            None => inside(outline, at) && !tpl.in_hole(at[0], at[1]),
                        };
                        if !shown {
                            continue;
                        }
                        let from_edge = (half[0] - q[0].abs()).min(half[1] - q[1].abs());
                        // 裏の柄は、裏から見て正しく読めるように、左右を逆にして描く。
                        let o = ((y - first) * w + x) * per;
                        f(y * w + x, &mut band[o..o + per], if back { [-q[0], q[1]] } else { q }, from_edge);
                    }
                }
            };
            let area = &mut target[y0 * w * per..y1 * w * per];
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(16);
            if (x1 - x0) * (y1 - y0) < 150_000 || threads < 2 {
                rows(y0, area);
                continue;
            }
            let band_rows = (y1 - y0).div_ceil(threads).max(8);
            let rows = &rows;
            std::thread::scope(|scope| {
                for (i, band) in area.chunks_mut(band_rows * w * per).enumerate() {
                    scope.spawn(move || rows(y0 + i * band_rows, band));
                }
            });
        }
    }

    /// タイルのように並べた柄の、かかる画素ごとに `f` を呼ぶ（引数は `each_pixel` と同じ。柄の座標は、そのタイルの中での座標）。
    /// 並びは、展開図の上で柄の向きにまっすぐ続く。置いたパネルだけか、展開図のすべての面に敷く。
    fn each_tiled_pixel<T: Send>(&self, decal: &Placed, tile: pakenoba_core::project::Tile, target: &mut [T], per: usize, f: impl Fn(usize, &mut [T], P2, f32) + Sync) {
        let (tpl, w, back, frame) = (self.tpl, self.w, self.back, &self.frame);
        let half = [decal.size[0] * 0.5, decal.size[1] * 0.5];
        let to_decal = decal.place(tpl).inverse();
        let grow = self.zone.map_or(0.0, |z| z.bleed);
        for (index, panel) in tpl.panels.iter().enumerate() {
            if !panel.takes_art() || (tile.panel_only && index != decal.panel) {
                continue;
            }
            let outline = &panel.outline;
            let lo = outline.iter().fold([f32::INFINITY; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
            let hi = outline.iter().fold([f32::NEG_INFINITY; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
            let (x0, x1, y0, y1) = frame.span([lo[0] - grow, lo[1] - grow], [hi[0] + grow, hi[1] + grow]);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            let rows = |first: usize, band: &mut [T]| {
                let count = band.len() / (w * per);
                let mut cuts: Vec<f32> = Vec::new();
                for y in first..first + count {
                    // 画素ごとに輪郭の内外を調べずに、行ごとに輪郭と交わる所を求めて、内側の区間だけを塗る（敷き詰めると範囲が広いので）。
                    let x_range: Vec<(usize, usize)> = if self.zone.is_some() {
                        vec![(x0, x1)]
                    } else {
                        let line = frame.at(0, y)[1];
                        cuts.clear();
                        for i in 0..outline.len() {
                            let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
                            if (a[1] > line) != (b[1] > line) {
                                cuts.push(a[0] + (line - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                            }
                        }
                        cuts.sort_by(f32::total_cmp);
                        cuts.chunks_exact(2)
                            .map(|pair| {
                                let from = (((pair[0] - frame.left) * frame.ppm - 0.5).ceil().max(0.0) as usize).max(x0);
                                let to = (((pair[1] - frame.left) * frame.ppm - 0.5).ceil().max(0.0) as usize).min(x1);
                                (from, to)
                            })
                            .filter(|(a, b)| a < b)
                            .collect()
                    };
                    for (from, to) in x_range {
                        for x in from..to {
                            let at = frame.at(x, y);
                            let shown = match self.zone {
                                // 塗り足しのあるとき: 紙の外は、いちばん近いパネルのぶんだけ（敷き詰めた柄は、どこでもかかっている）。
                                Some(zone) => inside(outline, at) || zone.reaches(y * w + x, index, at, |_| true),
                                None => !tpl.in_hole(at[0], at[1]),
                            };
                            if !shown {
                                continue;
                            }
                            let Some(q) = tile.wrap(to_decal.point(at), decal.size) else { continue };
                            let from_edge = (half[0] - q[0].abs()).min(half[1] - q[1].abs());
                            let o = ((y - first) * w + x) * per;
                            f(y * w + x, &mut band[o..o + per], if back { [-q[0], q[1]] } else { q }, from_edge);
                        }
                    }
                }
            };
            let area = &mut target[y0 * w * per..y1 * w * per];
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(16);
            if (x1 - x0) * (y1 - y0) < 150_000 || threads < 2 {
                rows(y0, area);
                continue;
            }
            let band_rows = (y1 - y0).div_ceil(threads).max(8);
            let rows = &rows;
            std::thread::scope(|scope| {
                for (i, band) in area.chunks_mut(band_rows * w * per).enumerate() {
                    scope.spawn(move || rows(y0 + i * band_rows, band));
                }
            });
        }
    }

    /// 並び `items`（下から。深さ `level` のものと、その下に入れ子になったもの）を、`target` に重ねる。
    /// `scale` は、外のグループ（通過）の不透明度。クリップしているものは、土台の見えている所にだけ重ねる。
    fn stack(&self, items: &[usize], target: &mut [u8], scale: f32, level: u8) {
        let mut k = 0;
        while k < items.len() {
            // 塊: 柄 1 つか、グループ（中身と見出し）。
            let start = k;
            while k < items.len() && depth(&self.decals[items[k]]) > level {
                k += 1;
            }
            let run = &items[start..k];
            let header = items.get(k).copied().filter(|&j| matches!(self.decals[j].picture, Picture::Group { .. }));
            let unit: (&[usize], Option<usize>) = if run.is_empty() {
                k += 1;
                (&[], Some(items[k - 1]))
            } else if let Some(h) = header {
                k += 1;
                (run, Some(h))
            } else {
                (run, None)
            };
            // 塊の上に続く、クリップしているもの。
            let clip_start = k;
            while k < items.len() && depth(&self.decals[items[k]]) == level && self.decals[items[k]].layer.clipped && !matches!(self.decals[items[k]].picture, Picture::Group { .. }) {
                k += 1;
            }
            let clipped = &items[clip_start..k];
            if clipped.is_empty() {
                self.unit(unit, target, scale, level);
                continue;
            }
            // 土台だけを描いて、見えている所（透明度）を覚えておく。
            let mut alone = vec![0u8; target.len()];
            self.unit(unit, &mut alone, 1.0, level);
            let clip: Vec<u8> = alone.chunks_exact(4).map(|p| p[3]).collect();
            self.unit(unit, target, scale, level);
            for &j in clipped {
                self.item(j, target, scale, Some(&clip));
            }
        }
    }

    /// 塊を 1 つ描く: 中身と見出しならグループ、中身だけ（見出しが見つからない）なら 1 つずつ、見出しだけなら空のグループ。
    fn unit(&self, (run, header): (&[usize], Option<usize>), target: &mut [u8], scale: f32, level: u8) {
        match header {
            Some(h) if matches!(self.decals[h].picture, Picture::Group { .. }) => self.group(run, h, target, scale, level),
            Some(h) => self.item(h, target, scale, None),
            None => self.stack(run, target, scale, level + 1),
        }
    }

    fn group(&self, run: &[usize], header: usize, target: &mut [u8], scale: f32, level: u8) {
        let layer = &self.decals[header].layer;
        if layer.hidden || run.is_empty() {
            return;
        }
        let opacity = layer.opacity.min(100) as f32 / 100.0 * scale;
        if layer.blend == Blend::PassThrough {
            // 通過: 中身を、そのまま外へ重ねる（グループの不透明度は、中身それぞれに掛ける）。
            self.stack(run, target, opacity, level + 1);
            return;
        }
        // 中身を透明な絵に重ねてから、グループの描画モードと不透明度で、外へ重ねる。
        let mut buffer = vec![0u8; target.len()];
        self.stack(run, &mut buffer, 1.0, level + 1);
        for (dst, src) in target.chunks_exact_mut(4).zip(buffer.chunks_exact(4)) {
            if src[3] > 0 {
                crate::blend::over(dst, [src[0] as f32, src[1] as f32, src[2] as f32], src[3] as f32 / 255.0 * opacity, layer.blend);
            }
        }
    }

    /// レイヤー 1 つ。`clip` があれば、その透明度（展開図の画素ごと）の所にだけ重ねる。
    fn item(&self, index: usize, target: &mut [u8], scale: f32, clip: Option<&[u8]>) {
        let placed = &self.decals[index];
        if placed.layer.hidden {
            return;
        }
        match placed.picture {
            Picture::Group { .. } => {}
            Picture::Adjustment => adjust_below(placed, target, scale, clip),
            _ => self.artwork(placed, target, scale, clip),
        }
    }

    fn artwork(&self, decal: &Placed, target: &mut [u8], scale: f32, clip: Option<&[u8]>) {
        // 塗りも、不透明度と同じく掛ける（レイヤースタイルがないので、Photoshop と同じ見た目）。
        let opacity = decal.layer.opacity.min(100) as f32 / 100.0 * decal.layer.fill.min(100) as f32 / 100.0 * scale;
        let picture = crate::effects::shown(decal, self.wait);
        let mask = decal.layer.mask.as_ref().filter(|m| m.enabled);
        let size = decal.size;
        let ppm = self.frame.ppm;
        let blend = decal.layer.blend;
        // 敷き詰めた柄は、展開図の全体にかかって重いので、画面に描くときは、描いた絵を覚えておいて重ねるだけにする
        // （ほかの柄を動かしている間に、毎回描き直さない）。
        if let Some(tile) = decal.layer.tile.filter(|t| !self.wait && !t.off) {
            let key = format!(
                "{}|{:?}|{tile:?}|{:?}|{:?}|{}|{:p}|{:?}{:?}{}",
                crate::effects::picture_key(&picture, size),
                decal.decal,
                mask.map(|m| (Arc::as_ptr(&m.data) as usize, m.invert)),
                self.frame,
                self.back,
                self.tpl,
                self.tpl.min,
                self.tpl.max,
                self.zone.is_some()
            );
            let layer = tiled_layer(&key, || {
                let mut layer = vec![0u8; self.w * self.frame.h * 4];
                self.each_pixel(decal, &mut layer, 4, true, |_, px, q, _| {
                    let color = picture.sample_at(q, size, ppm);
                    let mut a = color[3];
                    if let Some(mask) = mask {
                        a *= mask_at(mask, q, size);
                    }
                    px.copy_from_slice(&[color[0].round().clamp(0.0, 255.0) as u8, color[1].round().clamp(0.0, 255.0) as u8, color[2].round().clamp(0.0, 255.0) as u8, (a * 255.0).round().clamp(0.0, 255.0) as u8]);
                });
                layer
            });
            // 重ねるのも、行の束に分けて手分けする。
            let compose = |first: usize, band: &mut [u8]| {
                let from = &layer[first * 4..first * 4 + band.len()];
                for (k, (px, src)) in band.chunks_exact_mut(4).zip(from.chunks_exact(4)).enumerate() {
                    if src[3] == 0 {
                        continue;
                    }
                    let mut a = src[3] as f32 / 255.0 * opacity;
                    if let Some(clip) = clip {
                        a *= clip[first + k] as f32 / 255.0;
                    }
                    crate::blend::over(px, [src[0] as f32, src[1] as f32, src[2] as f32], a, blend);
                }
            };
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(16);
            let chunk = (target.len() / 4).div_ceil(threads).max(4096) * 4;
            let compose = &compose;
            std::thread::scope(|scope| {
                for (i, band) in target.chunks_mut(chunk).enumerate() {
                    scope.spawn(move || compose(i * chunk / 4, band));
                }
            });
            return;
        }
        self.each_pixel(decal, target, 4, true, |i, px, q, _| {
            let color = picture.sample_at(q, size, ppm);
            let mut a = color[3] * opacity;
            if let Some(mask) = mask {
                a *= mask_at(mask, q, size);
            }
            if let Some(clip) = clip {
                a *= clip[i] as f32 / 255.0;
            }
            crate::blend::over(px, [color[0], color[1], color[2]], a, blend);
        });
    }

    /// 選んでいる柄の枠と、カーソルの下の柄の枠。描画モードやグループに関係なく、いちばん上に描く。
    fn marks(&self, decal: &Placed, picked: bool, target: &mut [u8]) {
        // 枠は、柄の見えている部分に付ける（透明な余白は囲まない）。
        let area = crate::app::visible::of(decal);
        let mut seen = decal.clone();
        seen.decal = crate::app::visible::trim(&decal.decal, self.tpl, area).0;
        let decal = &seen;
        self.each_pixel(decal, target, 4, false, |_, px, q, from_edge| {
            let color = if picked && from_edge < 0.6 {
                // 選んでいる柄の枠。縁から 0.6 mm を、点線で描く。
                let dash = ((q[0] + q[1]) / 3.0).rem_euclid(2.0) < 1.0;
                if dash { [255.0, 255.0, 255.0, 1.0] } else { [20.0, 30.0, 40.0, 1.0] }
            } else if !picked && from_edge < 1.2 {
                HOVER
            } else {
                return;
            };
            crate::blend::over(px, [color[0], color[1], color[2]], color[3], Blend::Normal);
        });
    }
}

/// マスクの、柄の座標 `q` での値（0〜1）。
pub fn mask_at(mask: &pakenoba_core::project::Mask, q: P2, size: P2) -> f32 {
    let (w, h) = (mask.w as usize, mask.h as usize);
    let fx = ((q[0] / size[0] + 0.5) * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let fy = ((0.5 - q[1] / size[1]) * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let v = |x: usize, y: usize| mask.data[y * w + x] as f32;
    let m = ((v(x0, y0) * (1.0 - tx) + v(x1, y0) * tx) * (1.0 - ty) + (v(x0, y1) * (1.0 - tx) + v(x1, y1) * tx) * ty) / 255.0;
    if mask.invert { 1.0 - m } else { m }
}

/// 調整レイヤーを、下の絵（`target`）に効かせる。透明な所はそのまま。透明度は変えない。
pub fn adjust_below(placed: &Placed, target: &mut [u8], scale: f32, clip: Option<&[u8]>) {
    let stack: Vec<_> = crate::effects::enabled(&placed.layer.adjust).into_iter().filter(|k| k.per_pixel()).collect();
    let opacity = placed.layer.opacity.min(100) as f32 / 100.0 * placed.layer.fill.min(100) as f32 / 100.0 * scale;
    if stack.is_empty() || opacity <= 0.0 {
        return;
    }
    let mode = placed.layer.blend;
    let ops = crate::effects::prepare(&stack);
    for (n, px) in target.chunks_exact_mut(4).enumerate() {
        let opacity = opacity * clip.map_or(1.0, |c| c[n] as f32 / 255.0);
        if px[3] == 0 || opacity <= 0.0 {
            continue;
        }
        let below = [px[0] as f32, px[1] as f32, px[2] as f32];
        let adjusted = crate::effects::run(&ops, below).map(|v| v / 255.0);
        let b = below.map(|v| v / 255.0);
        let mixed = if mode == Blend::Normal { adjusted } else { crate::blend::mix(mode, b, adjusted) };
        for i in 0..3 {
            px[i] = ((b[i] + (mixed[i] - b[i]) * opacity).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

/// 柄を、展開図の画像に描く。`selected` の柄には、枠を付ける。
/// 描くのは、`back` で指定した面（表か裏）の柄だけ。隠した柄は描かない。グループと調整レイヤーも、ここで効かせる。
pub fn paint(surface: &Surface, tpl: &Template, decals: &[Placed], marks: Marks, back: bool, canvas: Canvas) {
    let Canvas { w, h, rgba, wait, frame, zone } = canvas;
    let frame = frame.unwrap_or_else(|| Frame::screen(tpl));
    debug_assert_eq!((frame.w, frame.h), (w, h));
    let painter = Painter { surface, tpl, decals, back, w, frame, wait, zone };
    let items: Vec<usize> = (0..decals.len()).filter(|&i| decals[i].back == back).collect();
    painter.stack(&items, rgba, 1.0, 0);
    for &index in &items {
        let picked = marks.selected.contains(&index);
        let hovered = marks.hover == Some(index) && !picked;
        if (picked || hovered) && is_artwork(&decals[index].picture) && shown_in_group(decals, index) {
            painter.marks(&decals[index], picked, rgba);
        }
    }
}

/// 柄 1 つのマスクを、展開図の大きさの白黒の絵にする（PSD のレイヤーマスク用。柄のない所は 0）。
pub fn paint_mask(surface: &Surface, tpl: &Template, decal: &Placed, back: bool, frame: Frame) -> Option<Vec<u8>> {
    let (w, h) = (frame.w, frame.h);
    let mask = decal.layer.mask.as_ref()?;
    let painter = Painter { surface, tpl, decals: std::slice::from_ref(decal), back, w, frame, wait: true, zone: None };
    let mut out = vec![0u8; w * h];
    painter.each_pixel(decal, &mut out, 1, true, |_, px, q, _| px[0] = (mask_at(mask, q, decal.size) * 255.0).round() as u8);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 左半分が赤、右半分が青の画像。
    fn red_blue() -> Picture {
        let (w, h) = (8, 2);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| if i % w < w / 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }).collect();
        Picture::Image { w, h, rgba: Arc::new(rgba), vector: false }
    }

    /// 展開図の画像の、展開図上の位置 `at` の色。
    fn pixel(tpl: &Template, w: usize, rgba: &[u8], at: P2) -> [u8; 4] {
        let (x, y) = (((at[0] - tpl.min[0]) * PX_PER_MM) as usize, ((tpl.max[1] - at[1]) * PX_PER_MM) as usize);
        let o = (y * w + x) * 4;
        [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]]
    }

    #[test]
    fn 裏の柄は裏の画像にだけ_左右を逆にして描く() {
        let tpl = Template::tuck_box(120.0, 60.0, 160.0, 0.5);
        let thin = Template::tuck_box(120.0, 60.0, 160.0, 0.02);
        let surface = Surface::new(&tpl, &thin);
        let (w, h) = (((tpl.max[0] - tpl.min[0]) * PX_PER_MM).ceil() as usize, ((tpl.max[1] - tpl.min[1]) * PX_PER_MM).ceil() as usize);
        let front = Placed { picture: red_blue(), decal: Decal { panel: 0, at: [0.5, 0.7], size: [40.0, 10.0], angle: 0.0 }, back: false, layer: Default::default() };
        let back = Placed { picture: red_blue(), decal: Decal { panel: 0, at: [0.5, 0.3], size: [40.0, 10.0], angle: 0.0 }, back: true, layer: Default::default() };
        let (front_at, back_at) = (front.center(&tpl), back.center(&tpl));
        let decals = [front, back];
        let left = |c: P2| [c[0] - 10.0, c[1]];
        let right = |c: P2| [c[0] + 10.0, c[1]];

        // 表の画像には、表の柄だけ。左が赤、右が青。
        let mut image = vec![0u8; w * h * 4];
        paint(&surface, &tpl, &decals, Marks::default(), false, Canvas::screen(w, h, &mut image));
        assert_eq!(pixel(&tpl, w, &image, left(front_at)), [255, 0, 0, 255]);
        assert_eq!(pixel(&tpl, w, &image, right(front_at)), [0, 0, 255, 255]);
        assert_eq!(pixel(&tpl, w, &image, back_at)[3], 0, "裏の柄は、表の画像に出ない");

        // 裏の画像には、裏の柄だけ。展開図の座標では左右が逆（裏返して見ると、左が赤になる）。
        let mut image = vec![0u8; w * h * 4];
        paint(&surface, &tpl, &decals, Marks::default(), true, Canvas::screen(w, h, &mut image));
        assert_eq!(pixel(&tpl, w, &image, left(back_at)), [0, 0, 255, 255]);
        assert_eq!(pixel(&tpl, w, &image, right(back_at)), [255, 0, 0, 255]);
        assert_eq!(pixel(&tpl, w, &image, front_at)[3], 0, "表の柄は、裏の画像に出ない");

        // 面ごとに取り出すと、もとの番号が付いてくる。
        assert_eq!(spots_on(&decals, false).0, vec![0]);
        assert_eq!(spots_on(&decals, true).0, vec![1]);
    }
}
