//! 箱を CPU で描く仮の描画器。
//!
//! 試作用。画面表示と動画書き出しで同じ絵を出すことと、GPU のない環境でも確認できることを優先した。
//! 本番の 3D 描画は wgpu で作り直す前提。

use crate::scene::{BoxDims, Camera, Projected, Projector, dot, faces, normalize, sub};

/// 貼り付ける絵柄。RGBA 8bit、アルファは乗算なし。
pub struct Texture {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// 描画結果。RGBA 8bit、不透明。
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

const PAPER: [f32; 3] = [245.0, 243.0, 238.0];
const BG_TOP: [f32; 3] = [58.0, 60.0, 66.0];
const BG_BOTTOM: [f32; 3] = [34.0, 35.0, 39.0];

/// 箱を描く。`ss` は縦横それぞれの過剰サンプリング倍率（輪郭のギザギザを減らす）。
pub fn render(dims: BoxDims, cam: Camera, tex: Option<&Texture>, out_w: usize, out_h: usize, ss: usize) -> Frame {
    let ss = ss.clamp(1, 4);
    let (out_w, out_h) = (out_w.max(2), out_h.max(2));
    let (w, h) = (out_w * ss, out_h * ss);
    let mut buf = vec![0u8; w * h * 3];

    for y in 0..h {
        let t = y as f32 / (h - 1) as f32;
        let c = [
            (BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t) as u8,
            (BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t) as u8,
            (BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t) as u8,
        ];
        for px in buf[y * w * 3..(y + 1) * w * 3].chunks_exact_mut(3) {
            px.copy_from_slice(&c);
        }
    }

    let proj = Projector::new(dims, cam, w as f32, h as f32);
    let light = normalize([0.35, 0.8, 0.55]);
    for face in faces(dims) {
        // 箱は凸なので、こちらを向いた面どうしは重ならない。裏向きの面を捨てれば奥行き比較は要らない。
        if dot(face.normal, sub(proj.eye(), face.corners[0])) <= 0.0 {
            continue;
        }
        let shade = 0.62 + 0.38 * dot(face.normal, light).max(0.0);
        let p = face.corners.map(|c| proj.project(c));
        let (u0, u1) = face.tex_u.unwrap_or((0.0, 0.0));
        let uv = [[u0, 0.0], [u1, 0.0], [u1, 1.0], [u0, 1.0]];
        let face_tex = if face.tex_u.is_some() { tex } else { None };
        for idx in [[0usize, 1, 2], [0, 2, 3]] {
            fill_triangle(&mut buf, w, h, [p[idx[0]], p[idx[1]], p[idx[2]]], [uv[idx[0]], uv[idx[1]], uv[idx[2]]], face_tex, shade);
        }
    }

    downsample(&buf, w, h, ss)
}

fn edge(a: Projected, b: Projected, x: f32, y: f32) -> f32 {
    (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
}

fn fill_triangle(buf: &mut [u8], w: usize, h: usize, p: [Projected; 3], uv: [[f32; 2]; 3], tex: Option<&Texture>, shade: f32) {
    let area = edge(p[0], p[1], p[2].x, p[2].y);
    if area.abs() < 1e-6 {
        return;
    }
    let min_x = p.iter().map(|v| v.x).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
    let max_x = (p.iter().map(|v| v.x).fold(f32::NEG_INFINITY, f32::max).ceil().max(0.0) as usize).min(w.saturating_sub(1));
    let min_y = p.iter().map(|v| v.y).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
    let max_y = (p.iter().map(|v| v.y).fold(f32::NEG_INFINITY, f32::max).ceil().max(0.0) as usize).min(h.saturating_sub(1));
    if min_x > max_x || min_y > max_y {
        return;
    }
    let inv_area = 1.0 / area;
    for y in min_y..=max_y {
        let fy = y as f32 + 0.5;
        for x in min_x..=max_x {
            let fx = x as f32 + 0.5;
            let b0 = edge(p[1], p[2], fx, fy) * inv_area;
            let b1 = edge(p[2], p[0], fx, fy) * inv_area;
            let b2 = edge(p[0], p[1], fx, fy) * inv_area;
            if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                continue;
            }
            let color = match tex {
                Some(t) => {
                    // 遠近補正: u/z と v/z と 1/z を画面上で線形に補間してから割り戻す。
                    let iz = b0 * p[0].inv_z + b1 * p[1].inv_z + b2 * p[2].inv_z;
                    let u = (b0 * uv[0][0] * p[0].inv_z + b1 * uv[1][0] * p[1].inv_z + b2 * uv[2][0] * p[2].inv_z) / iz;
                    let v = (b0 * uv[0][1] * p[0].inv_z + b1 * uv[1][1] * p[1].inv_z + b2 * uv[2][1] * p[2].inv_z) / iz;
                    sample(t, u, v)
                }
                None => PAPER,
            };
            let o = (y * w + x) * 3;
            buf[o] = (color[0] * shade).min(255.0) as u8;
            buf[o + 1] = (color[1] * shade).min(255.0) as u8;
            buf[o + 2] = (color[2] * shade).min(255.0) as u8;
        }
    }
}

/// 絵柄を双線形補間で読み、紙の地色の上に重ねた色を返す。
fn sample(t: &Texture, u: f32, v: f32) -> [f32; 3] {
    if t.w == 0 || t.h == 0 {
        return PAPER;
    }
    let x = (u.clamp(0.0, 1.0) * t.w as f32 - 0.5).clamp(0.0, (t.w - 1) as f32);
    let y = (v.clamp(0.0, 1.0) * t.h as f32 - 0.5).clamp(0.0, (t.h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(t.w - 1), (y0 + 1).min(t.h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |xx: usize, yy: usize| -> [f32; 4] {
        let o = (yy * t.w + xx) * 4;
        [t.rgba[o] as f32, t.rgba[o + 1] as f32, t.rgba[o + 2] as f32, t.rgba[o + 3] as f32 / 255.0]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let weights = [(1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy];
    // アルファを掛けてから補間する（透明部分の色がにじまないように）。
    let mut rgb = [0.0f32; 3];
    let mut alpha = 0.0f32;
    for (s, wgt) in [a, b, c, d].iter().zip(weights) {
        for i in 0..3 {
            rgb[i] += s[i] * s[3] * wgt;
        }
        alpha += s[3] * wgt;
    }
    [rgb[0] + PAPER[0] * (1.0 - alpha), rgb[1] + PAPER[1] * (1.0 - alpha), rgb[2] + PAPER[2] * (1.0 - alpha)]
}

fn downsample(buf: &[u8], w: usize, h: usize, ss: usize) -> Frame {
    let (ow, oh) = (w / ss, h / ss);
    let mut rgba = vec![255u8; ow * oh * 4];
    let n = (ss * ss) as u32;
    for oy in 0..oh {
        for ox in 0..ow {
            let mut sum = [0u32; 3];
            for sy in 0..ss {
                let row = (oy * ss + sy) * w;
                for sx in 0..ss {
                    let o = (row + ox * ss + sx) * 3;
                    sum[0] += buf[o] as u32;
                    sum[1] += buf[o + 1] as u32;
                    sum[2] += buf[o + 2] as u32;
                }
            }
            let o = (oy * ow + ox) * 4;
            rgba[o] = (sum[0] / n) as u8;
            rgba[o + 1] = (sum[1] / n) as u8;
            rgba[o + 2] = (sum[2] / n) as u8;
        }
    }
    Frame { w: ow, h: oh, rgba }
}
