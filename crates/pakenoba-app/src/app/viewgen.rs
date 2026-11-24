//! 生成AIに渡す、斜めから見た箱の画像（制御画像）と、生成した絵を面へ貼り戻す処理。
//!
//! いま立体の表示に見えている向きの箱を、同じカメラで、描いた絵（確認用）、線画、奥行き、面の向き、マスク、面の塗り分けに
//! して書き出す。ComfyUI の ControlNet、ブラウザ版の Firefly の構図の参照、Photoshop などに、そのまま渡せる。
//! カメラは `view.json` に書いておく。生成した絵（同じ縦横の比）を読むと、そのカメラで箱を描き直し、見えている面の画素ごとに
//! 展開図のどこかを求めて、展開図の絵（紙の表）に重ねる。四隅の台形補正ではなく、画素ごとに戻すので、曲面にも合う。

use pakenoba_i18n::t;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use pakenoba_core::math::{Mat4, V3, look_at};
use pakenoba_core::surface::panel_at;
use pakenoba_core::template::{P2, Template, Vertex, bounding_sphere};

use super::App;
use crate::art::{Frame, alpha_over, build_psd, make_sheet_art_at};

/// 書き出す画像の大きさ（幅、高さ）。
pub(super) const VIEW_SIZES: [[u32; 2]; 4] = [[1024, 768], [1536, 1152], [2048, 1536], [1024, 1024]];

/// 箱を描いたカメラ。`view.json` に書き、読み戻すときに同じ箱を描き直す。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ViewSpec {
    pub w: u32,
    pub h: u32,
    pub view: Mat4,
    pub proj: Mat4,
    /// 組み立ての度合いと、開け口の閉じ具合（`build_mesh_at` に渡す値）。
    pub fold: f32,
    pub closing: [f32; 2],
    /// 書き出したときの展開図の範囲（型や寸法が変わっていないかを見る）。
    pub min: P2,
    pub max: P2,
}

impl ViewSpec {
    pub fn to_json(&self) -> Value {
        json!({
            "format": "pakenoba-view",
            "version": 1,
            "size": [self.w, self.h],
            "view": self.view,
            "proj": self.proj,
            "fold": self.fold,
            "closing": self.closing,
            "dieline": { "min": self.min, "max": self.max },
            "maps": {
                "depth": "white is near, black is far (linear between the nearest and farthest point of the box)",
                "normal": "camera space: red +x right, green +y up, blue +z toward the camera; each mapped from -1..1 to 0..255",
                "lineart": "white lines on black: outline, folds and edges of the faces",
                "mask": "white where the box is",
                "faces": "one colour per face"
            }
        })
    }

    pub fn from_json(value: &Value) -> Result<ViewSpec, String> {
        let bad = || t!("This is not a view file written by PAKENOBA (view.json).").to_owned();
        if value.get("format").and_then(Value::as_str) != Some("pakenoba-view") {
            return Err(bad());
        }
        let mat = |key: &str| -> Result<Mat4, String> { serde_json::from_value(value.get(key).cloned().ok_or_else(bad)?).map_err(|_| bad()) };
        let size: [u32; 2] = serde_json::from_value(value.get("size").cloned().ok_or_else(bad)?).map_err(|_| bad())?;
        let dieline = value.get("dieline").ok_or_else(bad)?;
        Ok(ViewSpec {
            w: size[0],
            h: size[1],
            view: mat("view")?,
            proj: mat("proj")?,
            fold: value.get("fold").and_then(Value::as_f64).ok_or_else(bad)? as f32,
            closing: serde_json::from_value(value.get("closing").cloned().ok_or_else(bad)?).map_err(|_| bad())?,
            min: serde_json::from_value(dieline.get("min").cloned().ok_or_else(bad)?).map_err(|_| bad())?,
            max: serde_json::from_value(dieline.get("max").cloned().ok_or_else(bad)?).map_err(|_| bad())?,
        })
    }
}

/// 列ごとに持つ行列で、点を写す（`[x, y, z, w]`）。
fn apply(m: &Mat4, p: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    for (r, o) in out.iter_mut().enumerate() {
        *o = (0..4).map(|c| m[c][r] * p[c]).sum();
    }
    out
}

fn normalize3(v: V3) -> V3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// カメラから見た箱の、画素ごとの情報。
pub(crate) struct Raster {
    pub w: usize,
    pub h: usize,
    /// カメラからの距離（カメラの奥向き、mm）。箱のない所は無限大。
    pub depth: Vec<f32>,
    /// 外側の面なら、展開図の上の点（uv ではなく mm）。それ以外は NaN。
    pub at: Vec<P2>,
    /// 面の向き（カメラの座標）。
    pub normal: Vec<V3>,
    /// 0: 外側の面、1: 内側の面、2: 紙の断面、255: 箱がない。
    pub kind: Vec<u8>,
}

/// 三角形を 1 つずつ、奥行きを比べながら描く（カメラの座標での値を、遠近を正しく補間する）。
pub(crate) fn rasterize(mesh: &[Vertex], tpl: &Template, view: &Mat4, proj: &Mat4, w: usize, h: usize) -> Raster {
    let n = w * h;
    let mut r = Raster { w, h, depth: vec![f32::INFINITY; n], at: vec![[f32::NAN; 2]; n], normal: vec![[0.0; 3]; n], kind: vec![255; n] };
    let mut zbuf = vec![f32::INFINITY; n];
    let size = [tpl.max[0] - tpl.min[0], tpl.max[1] - tpl.min[1]];
    for tri in mesh.chunks_exact(3) {
        let mut sp = [[0.0f32; 3]; 3];
        let mut inv_w = [0.0f32; 3];
        let mut cam = [[0.0f32; 3]; 3];
        let mut behind = false;
        for (i, v) in tri.iter().enumerate() {
            let c = apply(view, [v.pos[0], v.pos[1], v.pos[2], 1.0]);
            let clip = apply(proj, c);
            if clip[3] <= 1e-6 {
                behind = true;
                break;
            }
            let iw = 1.0 / clip[3];
            sp[i] = [(clip[0] * iw + 1.0) * 0.5 * w as f32, (1.0 - clip[1] * iw) * 0.5 * h as f32, clip[2] * iw];
            inv_w[i] = iw;
            cam[i] = [c[0], c[1], c[2]];
        }
        if behind {
            continue;
        }
        let area = (sp[1][0] - sp[0][0]) * (sp[2][1] - sp[0][1]) - (sp[2][0] - sp[0][0]) * (sp[1][1] - sp[0][1]);
        if area.abs() < 1e-9 {
            continue;
        }
        let kind = tri[0].kind.round() as u8;
        let normal = normalize3([0, 1, 2].map(|k| (0..3).map(|c| view[c][k] * tri[0].normal[c]).sum::<f32>()));
        let x0 = sp.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let x1 = (sp.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max).ceil().max(0.0) as usize).min(w);
        let y0 = sp.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let y1 = (sp.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max).ceil().max(0.0) as usize).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let e = |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (py - a[1]) - (px - a[0]) * (b[1] - a[1]);
                let (b0, b1, b2) = (e(sp[1], sp[2]) / area, e(sp[2], sp[0]) / area, e(sp[0], sp[1]) / area);
                if b0 < -1e-4 || b1 < -1e-4 || b2 < -1e-4 {
                    continue;
                }
                let z = b0 * sp[0][2] + b1 * sp[1][2] + b2 * sp[2][2];
                let o = y * w + x;
                if z >= zbuf[o] {
                    continue;
                }
                zbuf[o] = z;
                // 遠近を正しく補間する重み。
                let (q0, q1, q2) = (b0 * inv_w[0], b1 * inv_w[1], b2 * inv_w[2]);
                let s = q0 + q1 + q2;
                let (q0, q1, q2) = (q0 / s, q1 / s, q2 / s);
                r.depth[o] = -(q0 * cam[0][2] + q1 * cam[1][2] + q2 * cam[2][2]);
                r.kind[o] = kind;
                r.normal[o] = normal;
                r.at[o] = if kind == 0 {
                    let u = q0 * tri[0].uv[0] + q1 * tri[1].uv[0] + q2 * tri[2].uv[0];
                    let v = q0 * tri[0].uv[1] + q1 * tri[1].uv[1] + q2 * tri[2].uv[1];
                    [tpl.min[0] + u * size[0], tpl.max[1] - v * size[1]]
                } else {
                    [f32::NAN; 2]
                };
            }
        }
    }
    r
}

/// 書き出す 6 枚の画像（RGBA）。
pub(crate) struct ControlSet {
    pub reference: Vec<u8>,
    pub lineart: Vec<u8>,
    pub depth: Vec<u8>,
    pub normal: Vec<u8>,
    pub mask: Vec<u8>,
    pub faces: Vec<u8>,
}

/// 面の番号ごとの、見分けやすい色（色相を黄金角ずつ回す）。
fn face_color(index: usize) -> [u8; 3] {
    let hue = (index as f32 * 137.508).rem_euclid(360.0) / 60.0;
    let x = 1.0 - (hue % 2.0 - 1.0).abs();
    let (r, g, b) = match hue as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [(r * 200.0 + 40.0) as u8, (g * 200.0 + 40.0) as u8, (b * 200.0 + 40.0) as u8]
}

/// 画素ごとの面（外側の面は面の番号、内側は 1000、断面は 1001、箱がなければ None）。
fn face_ids(r: &Raster, tpl: &Template) -> Vec<Option<usize>> {
    (0..r.w * r.h)
        .map(|o| match r.kind[o] {
            0 => panel_at(tpl, r.at[o]),
            1 => Some(1000),
            2 => Some(1001),
            _ => None,
        })
        .collect()
}

/// 面の境目と、奥行きの段差（輪郭）の画素。
fn edges(r: &Raster, ids: &[Option<usize>]) -> Vec<bool> {
    let (w, h) = (r.w, r.h);
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let o = y * w + x;
            for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                if nx >= w || ny >= h {
                    continue;
                }
                let q = ny * w + nx;
                let (d0, d1) = (r.depth[o], r.depth[q]);
                let step = d0.is_finite() != d1.is_finite() || (d0.is_finite() && (d0 - d1).abs() > 0.02 * d0.min(d1));
                if ids[o] != ids[q] || step {
                    out[o] = true;
                    out[q] = true;
                }
            }
        }
    }
    out
}

/// 範囲を `radius` 画素ぶん広げる。
fn grow(mask: &[bool], w: usize, h: usize, radius: usize) -> Vec<bool> {
    let mut cur = mask.to_vec();
    for _ in 0..radius {
        let prev = cur.clone();
        for y in 0..h {
            for x in 0..w {
                if prev[y * w + x] {
                    continue;
                }
                let near = (x > 0 && prev[y * w + x - 1]) || (x + 1 < w && prev[y * w + x + 1]) || (y > 0 && prev[(y - 1) * w + x]) || (y + 1 < h && prev[(y + 1) * w + x]);
                if near {
                    cur[y * w + x] = true;
                }
            }
        }
    }
    cur
}

/// 制御画像を作る。`art` は展開図の絵（`art_frame`。紙の色の上に柄を重ねた、確認用のもの）。
pub(crate) fn control_set(r: &Raster, tpl: &Template, art: &[u8], art_frame: &Frame) -> ControlSet {
    let n = r.w * r.h;
    let ids = face_ids(r, tpl);
    let edge = edges(r, &ids);
    // 線画は、1 画素の境目を、画像の大きさに合わせて少し太くする。
    let thick = grow(&edge, r.w, r.h, (r.w.max(r.h) / 1024).max(1));
    let finite: Vec<f32> = r.depth.iter().copied().filter(|d| d.is_finite()).collect();
    let near = finite.iter().copied().fold(f32::INFINITY, f32::min);
    let far = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max).max(near + 1e-3);
    let light = normalize3([-0.4, 0.6, 0.7]);
    let mut set = ControlSet { reference: vec![0; n * 4], lineart: vec![0; n * 4], depth: vec![0; n * 4], normal: vec![0; n * 4], mask: vec![0; n * 4], faces: vec![0; n * 4] };
    for o in 0..n {
        let px = o * 4;
        let line = if thick[o] { 255 } else { 0 };
        set.lineart[px..px + 4].copy_from_slice(&[line, line, line, 255]);
        let Some(id) = ids[o] else {
            set.reference[px..px + 4].copy_from_slice(&[255, 255, 255, 255]);
            set.depth[px..px + 4].copy_from_slice(&[0, 0, 0, 255]);
            set.normal[px..px + 4].copy_from_slice(&[128, 128, 255, 255]);
            set.mask[px..px + 4].copy_from_slice(&[0, 0, 0, 255]);
            set.faces[px..px + 4].copy_from_slice(&[0, 0, 0, 255]);
            continue;
        };
        let d = (255.0 * (far - r.depth[o]) / (far - near)).round().clamp(0.0, 255.0) as u8;
        set.depth[px..px + 4].copy_from_slice(&[d, d, d, 255]);
        let nrm = r.normal[o];
        // 裏向きの面（内側の面を外から見たときなど）も、カメラへ向けて塗る。
        let nrm = if nrm[2] < 0.0 { [-nrm[0], -nrm[1], -nrm[2]] } else { nrm };
        let enc = |v: f32| ((v * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
        set.normal[px..px + 4].copy_from_slice(&[enc(nrm[0]), enc(nrm[1]), enc(nrm[2]), 255]);
        set.mask[px..px + 4].copy_from_slice(&[255, 255, 255, 255]);
        let c = face_color(id);
        set.faces[px..px + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        // 確認用の絵: 外側の面は展開図の絵、内側は薄い灰色、断面は濃い灰色。光は斜め上から。
        let base: [f32; 3] = match r.kind[o] {
            0 => {
                let [fx, fy] = art_frame.to_px(r.at[o]);
                let (x, y) = ((fx.max(0.0) as usize).min(art_frame.w - 1), (fy.max(0.0) as usize).min(art_frame.h - 1));
                let a = (y * art_frame.w + x) * 4;
                [art[a] as f32, art[a + 1] as f32, art[a + 2] as f32]
            }
            1 => [215.0, 210.0, 200.0],
            _ => [150.0, 145.0, 140.0],
        };
        let shade = 0.55 + 0.45 * (nrm[0] * light[0] + nrm[1] * light[1] + nrm[2] * light[2]).max(0.0);
        let s = base.map(|v| (v * shade).round().clamp(0.0, 255.0) as u8);
        set.reference[px..px + 4].copy_from_slice(&[s[0], s[1], s[2], 255]);
    }
    set
}

/// 画像（RGBA、`iw` x `ih`）の、0〜1 の位置での色（近い 4 画素をなめらかにつなぐ）。
fn sample01(img: &[u8], iw: usize, ih: usize, u: f32, v: f32) -> [f32; 4] {
    let (fx, fy) = ((u * iw as f32 - 0.5).clamp(0.0, (iw - 1) as f32), (v * ih as f32 - 0.5).clamp(0.0, (ih - 1) as f32));
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(iw - 1), (y0 + 1).min(ih - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let px = |x: usize, y: usize| {
        let o = (y * iw + x) * 4;
        [img[o] as f32, img[o + 1] as f32, img[o + 2] as f32, img[o + 3] as f32]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut out = [0.0f32; 4];
    for i in 0..4 {
        out[i] = (a[i] * (1.0 - tx) + b[i] * tx) * (1.0 - ty) + (c[i] * (1.0 - tx) + d[i] * tx) * ty;
    }
    out
}

/// 生成した絵を、展開図の画像（`frame`）へ貼り戻した結果。
pub(crate) struct Projected {
    pub rgba: Vec<u8>,
    /// 貼った面（面の番号）。
    pub faces: Vec<usize>,
}

/// 生成した絵（`img`、`iw` x `ih`。書き出した画像と同じ縦横の比）を、`spec` のカメラで、見えている外側の面へ貼り戻す。
/// 面の境目と輪郭のすぐ近く（生成した絵に線や影が出やすい所）は使わず、同じ面の中で、まわりの色を広げて埋める。
/// カメラに対して浅すぎる面（真横に近い面）も使わない。`even` が真なら、面ごとの明るさを、いちばん正面を向いた面にそろえる。
#[allow(clippy::too_many_arguments)]
pub(crate) fn project(img: &[u8], iw: usize, ih: usize, spec: &ViewSpec, mesh: &[Vertex], tpl: &Template, frame: &Frame, even: bool) -> Projected {
    let (w, h) = (spec.w as usize, spec.h as usize);
    let r = rasterize(mesh, tpl, &spec.view, &spec.proj, w, h);
    let ids = face_ids(&r, tpl);
    let avoid = grow(&edges(&r, &ids), w, h, (w.max(h) / 512).max(2));
    let mut out = vec![0u8; frame.w * frame.h * 4];
    let mut owner = vec![u16::MAX; frame.w * frame.h];
    // 展開図の画素ごとに、カメラの画像の上で、その点が見えているかを調べる。
    // 外側の面の三角形を、展開図の画素へ描きながら、立体での位置をカメラへ写す。
    let size = [tpl.max[0] - tpl.min[0], tpl.max[1] - tpl.min[1]];
    for tri in mesh.chunks_exact(3).filter(|t| t[0].kind.round() as u8 == 0) {
        let to_mm = |uv: [f32; 2]| [tpl.min[0] + uv[0] * size[0], tpl.max[1] - uv[1] * size[1]];
        let sp: [[f32; 2]; 3] = [0, 1, 2].map(|i| frame.to_px(to_mm(tri[i].uv)));
        let area = (sp[1][0] - sp[0][0]) * (sp[2][1] - sp[0][1]) - (sp[2][0] - sp[0][0]) * (sp[1][1] - sp[0][1]);
        if area.abs() < 1e-9 {
            continue;
        }
        let centroid = to_mm([(tri[0].uv[0] + tri[1].uv[0] + tri[2].uv[0]) / 3.0, (tri[0].uv[1] + tri[1].uv[1] + tri[2].uv[1]) / 3.0]);
        let Some(panel) = panel_at(tpl, centroid) else { continue };
        let x0 = sp.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let x1 = (sp.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max).ceil().max(0.0) as usize).min(frame.w);
        let y0 = sp.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let y1 = (sp.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max).ceil().max(0.0) as usize).min(frame.h);
        let normal_cam = normalize3([0, 1, 2].map(|k| (0..3).map(|c| spec.view[c][k] * tri[0].normal[c]).sum::<f32>()));
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let e = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (py - a[1]) - (px - a[0]) * (b[1] - a[1]);
                let (b0, b1, b2) = (e(sp[1], sp[2]) / area, e(sp[2], sp[0]) / area, e(sp[0], sp[1]) / area);
                if b0 < -1e-4 || b1 < -1e-4 || b2 < -1e-4 {
                    continue;
                }
                let pos = [0, 1, 2].map(|k| b0 * tri[0].pos[k] + b1 * tri[1].pos[k] + b2 * tri[2].pos[k]);
                let cam = apply(&spec.view, [pos[0], pos[1], pos[2], 1.0]);
                let clip = apply(&spec.proj, cam);
                if clip[3] <= 1e-6 {
                    continue;
                }
                let (u, v) = ((clip[0] / clip[3] + 1.0) * 0.5, (1.0 - clip[1] / clip[3]) * 0.5);
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                let (sx, sy) = ((u * w as f32) as usize, (v * h as f32) as usize);
                let so = sy * w + sx;
                let dist = -cam[2];
                // 見えているか: カメラの画像のその画素に写っているのが、この面の外側で、奥行きも合う。
                // （面の名前まで比べる。閉じた箱の中の差し込みは、底のすぐ内側にあり、奥行きだけでは見分けられない。）
                if r.kind[so] != 0 || ids[so] != Some(panel) || dist > r.depth[so] + 0.8 + 0.004 * dist || avoid[so] {
                    continue;
                }
                // 真横に近い面は、生成した絵が引き伸ばされているので使わない。
                let to_eye = normalize3([-cam[0], -cam[1], -cam[2]]);
                let facing = (normal_cam[0] * to_eye[0] + normal_cam[1] * to_eye[1] + normal_cam[2] * to_eye[2]).abs();
                if facing < 0.25 {
                    continue;
                }
                let c = sample01(img, iw, ih, u, v);
                let o = y * frame.w + x;
                out[o * 4..o * 4 + 4].copy_from_slice(&[c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, c[3].round() as u8]);
                owner[o] = panel as u16;
            }
        }
    }
    // 使わなかった縁を、同じ面の中で、まわりの色を広げて埋める（面ごとの形の中だけ）。
    let inside_panel: Vec<u16> = {
        let mut m = vec![u16::MAX; frame.w * frame.h];
        for (panel, p) in tpl.panels.iter().enumerate() {
            let lo = p.outline.iter().fold([f32::MAX; 2], |m, q| [m[0].min(q[0]), m[1].min(q[1])]);
            let hi = p.outline.iter().fold([f32::MIN; 2], |m, q| [m[0].max(q[0]), m[1].max(q[1])]);
            let (x0, x1, y0, y1) = frame.span(lo, hi);
            for y in y0..y1 {
                for x in x0..x1 {
                    if pakenoba_core::template::inside(&p.outline, frame.at(x, y)) {
                        m[y * frame.w + x] = panel as u16;
                    }
                }
            }
        }
        m
    };
    let faces: Vec<usize> = {
        let mut f: Vec<usize> = owner.iter().filter(|&&p| p != u16::MAX).map(|&p| p as usize).collect();
        f.sort_unstable();
        f.dedup();
        f
    };
    // 埋めた画素から、同じ面の中の、まだ埋めていない画素へ、幅を数えながら広げる（近い順に 1 回ずつ）。
    let reach = ((3.0 * frame.ppm).ceil() as u32 + (w.max(h) / 256) as u32).max(2);
    let wanted = |o: usize| inside_panel[o] != u16::MAX && faces.contains(&(inside_panel[o] as usize));
    let mut steps = vec![0u32; frame.w * frame.h];
    let border = |o: usize| {
        let (x, y) = (o % frame.w, o / frame.w);
        [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)].into_iter().any(|(nx, ny)| nx < frame.w && ny < frame.h && owner[ny * frame.w + nx] == u16::MAX && wanted(ny * frame.w + nx))
    };
    let mut queue: std::collections::VecDeque<usize> = (0..frame.w * frame.h).filter(|&o| owner[o] != u16::MAX && border(o)).collect();
    while let Some(o) = queue.pop_front() {
        if steps[o] >= reach {
            continue;
        }
        let (x, y) = (o % frame.w, o / frame.w);
        for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
            if nx >= frame.w || ny >= frame.h {
                continue;
            }
            let q = ny * frame.w + nx;
            if owner[q] != u16::MAX || !wanted(q) || inside_panel[q] != owner[o] {
                continue;
            }
            let color = [out[o * 4], out[o * 4 + 1], out[o * 4 + 2], out[o * 4 + 3]];
            out[q * 4..q * 4 + 4].copy_from_slice(&color);
            owner[q] = owner[o];
            steps[q] = steps[o] + 1;
            queue.push_back(q);
        }
    }
    if even && faces.len() > 1 {
        even_faces(&mut out, &owner, &faces, mesh, spec, tpl);
    }
    Projected { rgba: out, faces }
}

/// 面ごとの明るさを、いちばん正面を向いた面にそろえる（生成した絵の、光の当たり方の違いを弱める）。
fn even_faces(out: &mut [u8], owner: &[u16], faces: &[usize], mesh: &[Vertex], spec: &ViewSpec, tpl: &Template) {
    let size = [tpl.max[0] - tpl.min[0], tpl.max[1] - tpl.min[1]];
    // 面ごとの向き（カメラへの向きの度合い）。
    let mut facing = vec![0.0f32; tpl.panels.len()];
    for tri in mesh.chunks_exact(3).filter(|t| t[0].kind.round() as u8 == 0) {
        let uv = [(tri[0].uv[0] + tri[1].uv[0] + tri[2].uv[0]) / 3.0, (tri[0].uv[1] + tri[1].uv[1] + tri[2].uv[1]) / 3.0];
        let Some(panel) = panel_at(tpl, [tpl.min[0] + uv[0] * size[0], tpl.max[1] - uv[1] * size[1]]) else { continue };
        let n = normalize3([0, 1, 2].map(|k| (0..3).map(|c| spec.view[c][k] * tri[0].normal[c]).sum::<f32>()));
        let pos = apply(&spec.view, [tri[0].pos[0], tri[0].pos[1], tri[0].pos[2], 1.0]);
        let to_eye = normalize3([-pos[0], -pos[1], -pos[2]]);
        facing[panel] = facing[panel].max((n[0] * to_eye[0] + n[1] * to_eye[1] + n[2] * to_eye[2]).abs());
    }
    let luma = |p: &[u8]| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32;
    // 面ごとの明るさの平均（先に全部求めておく）。
    let mut sums = vec![(0.0f32, 0usize); tpl.panels.len()];
    for (i, &o) in owner.iter().enumerate() {
        if (o as usize) < sums.len() {
            sums[o as usize].0 += luma(&out[i * 4..i * 4 + 4]);
            sums[o as usize].1 += 1;
        }
    }
    let mean = |panel: usize| sums[panel].0 / sums[panel].1.max(1) as f32;
    let Some(&key) = faces.iter().max_by(|&&a, &&b| facing[a].total_cmp(&facing[b])) else { return };
    let target = mean(key);
    for &panel in faces {
        if panel == key {
            continue;
        }
        let gain = (target / mean(panel).max(1.0)).clamp(0.7, 1.6);
        for (i, &o) in owner.iter().enumerate() {
            if o as usize == panel {
                for c in &mut out[i * 4..i * 4 + 3] {
                    *c = (*c as f32 * gain).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

impl App {
    /// いま立体の表示に見えている向きのカメラで、`size` の画像を描くためのもの。
    pub(crate) fn view_spec(&self, size: [u32; 2]) -> ViewSpec {
        let closing = [1.0 - self.open[0], 1.0 - self.open[1]];
        let mesh = self.template.build_mesh_at(self.fold, closing);
        let aspect = size[0] as f32 / size[1] as f32;
        let (center, radius) = bounding_sphere(&mesh);
        let fov = 30f32.to_radians();
        let view_aspect = self.solid_aspect;
        let dist = radius / (fov * 0.5).sin() * 1.1 / self.zoom / view_aspect.min(1.0);
        let share = if view_aspect > aspect { 1.0 } else { view_aspect / aspect };
        let fov_out = 2.0 * ((fov * 0.5).tan() * share).atan();
        let (dir, up) = super::views::orbit(self.yaw, self.pitch);
        let eye = [center[0] + dir[0] * dist, center[1] + dir[1] * dist, center[2] + dir[2] * dist];
        let view = look_at(eye, center, up);
        let proj = super::views::projection(self.ortho, fov_out, aspect, dist, radius);
        ViewSpec { w: size[0], h: size[1], view, proj, fold: self.fold, closing, min: self.template.min, max: self.template.max }
    }

    /// 制御画像と `view.json` を、フォルダ `dir` に書く。書いたファイルを返す。
    pub(crate) fn write_view(&self, dir: &Path, spec: &ViewSpec) -> Result<Vec<PathBuf>, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mesh = self.template.build_mesh_at(spec.fold, spec.closing);
        let r = rasterize(&mesh, &self.template, &spec.view, &spec.proj, spec.w as usize, spec.h as usize);
        let frame = self.sheet.frame;
        let art = crate::art::compose(&self.sheet, self.custom[0].as_deref(), false, |rgba| crate::paint::paint(&self.surface, &self.template, &self.decals, crate::paint::Marks::default(), false, crate::paint::Canvas::output(frame.w, frame.h, rgba)));
        let set = control_set(&r, &self.template, &art, &frame);
        let mut files = Vec::new();
        for (name, rgba) in [("reference", &set.reference), ("lineart", &set.lineart), ("depth", &set.depth), ("normal", &set.normal), ("mask", &set.mask), ("faces", &set.faces)] {
            let path = dir.join(format!("{name}.png"));
            std::fs::write(&path, super::outputs::png_bytes(spec.w, spec.h, rgba)?).map_err(|e| e.to_string())?;
            files.push(path);
        }
        let path = dir.join("view.json");
        std::fs::write(&path, serde_json::to_string_pretty(&spec.to_json()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        files.push(path);
        Ok(files)
    }

    /// 生成した絵（`image_path`）を、`spec` のカメラで面へ貼り戻し、紙の表の展開図の絵に重ねる。元に戻せる。
    /// 戻せた面の数を返す。
    pub(crate) fn apply_view_image(&mut self, image_path: &Path, spec: &ViewSpec, even: bool) -> Result<usize, String> {
        let same = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
        if !same(spec.min, self.template.min) || !same(spec.max, self.template.max) {
            return Err(t!("The view file does not match the current box (the shape or the dimensions have changed since it was written).").to_owned());
        }
        let bytes = std::fs::read(image_path).map_err(|e| t!("Could not read {}: {e}", image_path.display()))?;
        let crate::paint::Picture::Image { w: iw, h: ih, rgba, .. } = super::decode_image(&bytes)? else {
            return Err(t!("Could not read this as an image (PNG, JPEG, PSD and SVG are supported)").to_owned());
        };
        let (want, got) = (spec.w as f32 / spec.h as f32, iw as f32 / ih as f32);
        if (want / got - 1.0).abs() > 0.02 {
            return Err(t!("The image is {iw} x {ih}, but the view was {} x {}. Use an image with the same width-to-height ratio (generate at the same size, or scale it).", spec.w, spec.h));
        }
        let mesh = self.template.build_mesh_at(spec.fold, spec.closing);
        // 生成した絵は、箱全体で 1000〜2000 画素ほど。展開図へは 200 dpi までで貼れば足りる（それより細かくしても絵は細かくならない）。
        let export = self.export_frame();
        let frame = if export.dpi_value() > 200.0 { Frame::dpi(&self.template, 200.0, self.output.bleed.max(0.0)) } else { export };
        let projected = project(&rgba, iw, ih, spec, &mesh, &self.template, &frame, even);
        if projected.faces.is_empty() {
            return Err(t!("No face of the box could be found in the image.").to_owned());
        }
        // いまの展開図の絵（あれば）の上に重ねる。
        let mut art = self.bases[0].as_ref().map_or_else(|| vec![0u8; frame.w * frame.h * 4], |base| base.fitted(&self.template, frame));
        alpha_over(&mut art, &projected.rgba);
        let sheet = make_sheet_art_at(&self.template, frame);
        let guides = t!("{guides} (cut lines, fold lines)", guides = pakenoba_i18n::tr(crate::art::GUIDE_LAYER_PREFIX));
        let psd = build_psd(frame.w, frame.h, frame.dpi_value(), (pakenoba_i18n::tr("Artwork"), art), Vec::new(), (guides, sheet.lines), None)?;
        let stem = image_path.file_stem().map_or_else(|| t!("Generated").to_owned(), |s| s.to_string_lossy().into_owned());
        let name = image_path.with_file_name(t!("{stem} (on the faces).psd"));
        self.load_base(&name, psd, 0);
        Ok(projected.faces.len())
    }
}

/// 画像の隣にある `view.json`（なければ、そのフォルダの上の階層も見る）。
pub(crate) fn find_view_json(image: &Path) -> Option<PathBuf> {
    let dir = image.parent()?;
    [dir.join("view.json"), dir.parent()?.join("view.json")].into_iter().find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    fn black() -> App {
        let mut app = App::new(&Initial::default());
        let (_, text) = super::super::samples::SAMPLES[3];
        let opened = pakenoba_core::project::Project::from_text(text).unwrap();
        assert!(app.apply_project(opened.project).is_empty());
        app.output.dpi = 100;
        app
    }

    #[test]
    fn 制御画像と_カメラを書き出し_カメラは読み戻せる() {
        let app = black();
        let dir = std::env::temp_dir().join(format!("pakenoba-view-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let spec = app.view_spec([512, 384]);
        let files = app.write_view(&dir, &spec).unwrap();
        assert_eq!(files.len(), 7);
        let json: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("view.json")).unwrap()).unwrap();
        assert_eq!(ViewSpec::from_json(&json).unwrap(), spec);
        let mesh = app.template.build_mesh_at(spec.fold, spec.closing);
        let r = rasterize(&mesh, &app.template, &spec.view, &spec.proj, 512, 384);
        let boxed = r.kind.iter().filter(|&&k| k != 255).count();
        // 箱は画像のまん中にあり、画像の 1 割より大きく、全部ではない。
        assert!(boxed > 512 * 384 / 10 && boxed < 512 * 384, "{boxed}");
        assert_eq!(r.kind[(384 / 2) * 512 + 256], 0, "まん中は外側の面");
        assert_eq!(r.kind[0], 255, "隅は箱の外");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 面ごとに塗った絵を貼り戻すと_展開図のその面に戻る() {
        let mut app = black();
        let dir = std::env::temp_dir().join(format!("pakenoba-view-back-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let spec = app.view_spec([768, 576]);
        app.write_view(&dir, &spec).unwrap();
        // 「生成した絵」として、面ごとの塗り分けをそのまま使う（倍の大きさにして、大きさが違っても合うことも見る）。
        let faces = std::fs::read(dir.join("faces.png")).unwrap();
        let crate::paint::Picture::Image { w, h, rgba, .. } = super::super::decode_image(&faces).unwrap() else { panic!() };
        let big: Vec<u8> = (0..w * 2 * h * 2).flat_map(|i| { let (x, y) = (i % (w * 2) / 2, i / (w * 2) / 2); let o = (y * w + x) * 4; [rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]] }).collect();
        let path = dir.join("generated.png");
        std::fs::write(&path, super::super::outputs::png_bytes((w * 2) as u32, (h * 2) as u32, &big).unwrap()).unwrap();
        let count = app.apply_view_image(&path, &spec, false).unwrap();
        assert!(count >= 2, "{count}");
        let base = app.bases[0].as_ref().unwrap();
        assert!(base.file.name.contains("generated"));
        let custom = app.custom[0].as_ref().unwrap();
        let frame = Frame::screen(&app.template);
        let mut matched = 0;
        for (index, panel) in app.template.panels.iter().enumerate() {
            let n = panel.outline.len() as f32;
            let c = panel.outline.iter().fold([0.0f32; 2], |s, p| [s[0] + p[0] / n, s[1] + p[1] / n]);
            let [x, y] = frame.to_px(c);
            let o = (y as usize * frame.w + x as usize) * 4;
            if custom[o + 3] == 0 {
                continue;
            }
            let want = face_color(index);
            for k in 0..3 {
                assert!((custom[o + k] as i32 - want[k] as i32).abs() <= 6, "{} に {:?}（{want:?} のはず）", panel.name, &custom[o..o + 4]);
            }
            matched += 1;
        }
        assert!(matched >= 2, "{matched}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
