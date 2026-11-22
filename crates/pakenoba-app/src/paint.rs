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
    decals.iter().enumerate().filter(|(_, p)| p.back == back && !p.layer.hidden && !p.layer.locked).map(|(index, p)| (index, p.decal)).unzip()
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

/// 描き込む先の、展開図の画像（RGBA、左上が展開図の左上）。
pub struct Canvas<'a> {
    pub w: usize,
    pub h: usize,
    pub rgba: &'a mut [u8],
}

/// 柄を、展開図の画像に描く。`selected` の柄には、枠を付ける。
/// 描くのは、`back` で指定した面（表か裏）の柄だけ。隠した柄は描かない。
pub fn paint(surface: &Surface, tpl: &Template, decals: &[Placed], selected: Option<usize>, back: bool, canvas: Canvas) {
    let Canvas { w, h, rgba } = canvas;
    for (index, decal) in decals.iter().enumerate().filter(|(_, decal)| decal.back == back && !decal.layer.hidden) {
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
                    // 裏の柄は、裏から見て正しく読めるように、左右を逆にして描く。
                    let mut color = decal.picture.sample(if back { [-q[0], q[1]] } else { q }, decal.size);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 左半分が赤、右半分が青の画像。
    fn red_blue() -> Picture {
        let (w, h) = (8, 2);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| if i % w < w / 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }).collect();
        Picture::Image { w, h, rgba: Arc::new(rgba) }
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
        paint(&surface, &tpl, &decals, None, false, Canvas { w, h, rgba: &mut image });
        assert_eq!(pixel(&tpl, w, &image, left(front_at)), [255, 0, 0, 255]);
        assert_eq!(pixel(&tpl, w, &image, right(front_at)), [0, 0, 255, 255]);
        assert_eq!(pixel(&tpl, w, &image, back_at)[3], 0, "裏の柄は、表の画像に出ない");

        // 裏の画像には、裏の柄だけ。展開図の座標では左右が逆（裏返して見ると、左が赤になる）。
        let mut image = vec![0u8; w * h * 4];
        paint(&surface, &tpl, &decals, None, true, Canvas { w, h, rgba: &mut image });
        assert_eq!(pixel(&tpl, w, &image, left(back_at)), [0, 0, 255, 255]);
        assert_eq!(pixel(&tpl, w, &image, right(back_at)), [255, 0, 0, 255]);
        assert_eq!(pixel(&tpl, w, &image, front_at)[3], 0, "表の柄は、裏の画像に出ない");

        // 面ごとに取り出すと、もとの番号が付いてくる。
        assert_eq!(spots_on(&decals, false).0, vec![0]);
        assert_eq!(spots_on(&decals, true).0, vec![1]);
    }
}
