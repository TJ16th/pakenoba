//! 柄の絵と、展開図の画像への描き込み。
//!
//! 柄の場所と、隣の面への続き方は、中核（`pakenoba_core::surface`）が決める。ここは、絵を画素にするところ。
//! いまは展開図の画像に描き込んでいる。あとで、柄ごとに元の解像度のまま描く形に置き換える。

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use pakenoba_core::surface::{Decal, Surface};
use pakenoba_core::template::{P2, Template, inside};

use crate::art::PX_PER_MM;

/// 立体の上に置いた柄。場所と大きさ（`Decal`）に、絵を添えたもの。
#[derive(Clone)]
pub struct Placed {
    pub decal: Decal,
    pub picture: Picture,
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

/// 柄の場所だけを取り出す（中核の関数に渡すため）。
pub fn spots(decals: &[Placed]) -> Vec<Decal> {
    decals.iter().map(|p| p.decal).collect()
}

/// 柄の絵。
#[derive(Clone)]
pub enum Picture {
    /// 丸いロゴ（確認用。上向きの矢印で、向きが分かる）。
    Logo,
    /// 帯（確認用。右向きの山形の並びで、向きと続き方が分かる）。
    Band,
    /// 読み込んだ画像（RGBA、アルファは乗算なし）。
    Image { w: usize, h: usize, rgba: Arc<Vec<u8>> },
}

impl Picture {
    pub fn label(&self) -> &'static str {
        match self {
            Picture::Logo => "ロゴ",
            Picture::Band => "帯",
            Picture::Image { .. } => "画像",
        }
    }

    /// 柄の座標 `q`（中心が原点、mm、上が正）での色。RGB は 0〜255、アルファは 0〜1。
    pub fn sample(&self, q: P2, size: P2) -> [f32; 4] {
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
                let alpha = ((1.0 - rho) * radius * PX_PER_MM + 0.5).clamp(0.0, 1.0);
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
            Picture::Image { w, h, rgba } => {
                // 画素の中心どうしの間を、なめらかにつなぐ。
                let fx = ((q[0] + half[0]) / size[0] * *w as f32 - 0.5).clamp(0.0, (*w - 1) as f32);
                let fy = ((half[1] - q[1]) / size[1] * *h as f32 - 0.5).clamp(0.0, (*h - 1) as f32);
                let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
                let (x1, y1) = ((x0 + 1).min(*w - 1), (y0 + 1).min(*h - 1));
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                let texel = |x: usize, y: usize| {
                    let o = (y * *w + x) * 4;
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
    }
}

/// 柄を、展開図の画像（RGBA、左上が展開図の左上）に描く。`selected` の柄には、枠を付ける。
pub fn paint(surface: &Surface, tpl: &Template, decals: &[Placed], selected: Option<usize>, w: usize, h: usize, rgba: &mut [u8]) {
    for (index, decal) in decals.iter().enumerate() {
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
            let (lo, hi) = ([lo_d[0].max(lo_p[0]), lo_d[1].max(lo_p[1])], [hi_d[0].min(hi_p[0]), hi_d[1].min(hi_p[1])]);
            if lo[0] >= hi[0] || lo[1] >= hi[1] {
                continue;
            }
            let x0 = ((lo[0] - tpl.min[0]) * PX_PER_MM).floor().max(0.0) as usize;
            let x1 = (((hi[0] - tpl.min[0]) * PX_PER_MM).ceil().max(0.0) as usize).min(w);
            let y0 = ((tpl.max[1] - hi[1]) * PX_PER_MM).floor().max(0.0) as usize;
            let y1 = (((tpl.max[1] - lo[1]) * PX_PER_MM).ceil().max(0.0) as usize).min(h);
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = [tpl.min[0] + (x as f32 + 0.5) / PX_PER_MM, tpl.max[1] - (y as f32 + 0.5) / PX_PER_MM];
                    let q = to_decal.point(at);
                    if q[0].abs() > half[0] || q[1].abs() > half[1] || !inside(outline, at) || tpl.in_hole(at[0], at[1]) {
                        continue;
                    }
                    let mut color = decal.picture.sample(q, decal.size);
                    if selected == Some(index) {
                        // 選んでいる柄の枠。縁から 0.6 mm を、点線で描く。
                        let from_edge = (half[0] - q[0].abs()).min(half[1] - q[1].abs());
                        let dash = ((q[0] + q[1]) / 3.0).rem_euclid(2.0) < 1.0;
                        if from_edge < 0.6 {
                            color = if dash { [255.0, 255.0, 255.0, 1.0] } else { [20.0, 30.0, 40.0, 1.0] };
                        }
                    }
                    let o = (y * w + x) * 4;
                    let a = color[3];
                    if a <= 0.0 {
                        continue;
                    }
                    let below = rgba[o + 3] as f32 / 255.0;
                    let out_a = a + below * (1.0 - a);
                    for i in 0..3 {
                        rgba[o + i] = ((color[i] * a + rgba[o + i] as f32 * below * (1.0 - a)) / out_a).round() as u8;
                    }
                    rgba[o + 3] = (out_a * 255.0).round() as u8;
                }
            }
        }
    }
}
