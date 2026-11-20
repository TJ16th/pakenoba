//! 型紙（パネルと折り線）と、厚みのある紙を折る計算。
//!
//! 座標は「紙の座標」(u, v, n) で持つ。u と v は展開図上の位置（mm）、n は紙の厚み方向で、
//! n = 0 が外側（印刷面）、n = 厚み が内側。谷折りは内側どうしが近づく向きで、折りの軸は内側の面に置く。
//! 外側にできるすき間は、折り線ごとに円弧の面で埋める。

use bytemuck::{Pod, Zeroable};

use crate::math::{Affine, V3, add, cross, dot, normalize, rotate, sub};

pub type P2 = [f32; 2];

/// 親パネルとのつながり（折り線）。
pub struct Hinge {
    pub parent: usize,
    /// 折り線の両端（展開図上）。
    pub a: P2,
    pub b: P2,
    /// 折りきったときの角度（ラジアン）。正が谷折り。
    pub angle: f32,
    /// 折りが進む区間。全体の進み具合 0〜1 のうち、どこからどこまでで折るか。
    pub window: (f32, f32),
}

/// 紙の一区画。輪郭は凸多角形で、反時計回り。
pub struct Panel {
    pub name: &'static str,
    pub outline: Vec<P2>,
    pub hinge: Option<Hinge>,
    /// 組み上がると隠れる部分（のりしろ、差し込みなど）か。絵柄の色分けに使う。
    pub hidden_when_closed: bool,
}

/// 型紙。パネルは親が子より先に並ぶ。
pub struct Template {
    pub panels: Vec<Panel>,
    pub thickness: f32,
    pub min: P2,
    pub max: P2,
    /// 根のパネル（正面）の中心と、箱の内寸の奥行き。表示用の座標へ移すときに使う。
    root_center: P2,
    depth: f32,
    /// 確認用の絵柄で、折り線をまたぐ位置に置く円（中心と半径）。
    pub marks: Vec<(P2, f32)>,
}

/// 描画用の頂点。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// 0: 外側（絵柄を貼る）、1: 内側、2: 紙の断面。
    pub kind: f32,
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn signed_area(points: &[P2]) -> f32 {
    let mut area = 0.0;
    for i in 0..points.len() {
        let (p, q) = (points[i], points[(i + 1) % points.len()]);
        area += p[0] * q[1] - q[0] * p[1];
    }
    area * 0.5
}

fn ccw(mut points: Vec<P2>) -> Vec<P2> {
    if signed_area(&points) < 0.0 {
        points.reverse();
    }
    points
}

fn centroid(points: &[P2]) -> P2 {
    let n = points.len() as f32;
    [points.iter().map(|p| p[0]).sum::<f32>() / n, points.iter().map(|p| p[1]).sum::<f32>() / n]
}

fn rect(u0: f32, v0: f32, u1: f32, v1: f32) -> Vec<P2> {
    ccw(vec![[u0, v0], [u1, v0], [u1, v1], [u0, v1]])
}

impl Template {
    /// 差し込み式の箱（キャラメル箱）。寸法は内寸（mm）。
    ///
    /// 厚みの扱いは単純なルールで、重なる部分を紙 1 枚ぶんずらす。
    /// - のりしろは背面の内側に入る。そのぶん左側面を 1 枚ぶん短く、背面を 1 枚ぶん長くする。
    /// - フタは差し込みフラップ（左右の小さいフタ）の上に乗る。そのぶん正面と背面を上下に 1 枚ぶんずつ高くする。
    /// - 差し込み（ベロ）は正面の内側に入る。そのぶんフタを 1 枚ぶん短くする。
    pub fn tuck_box(w: f32, d: f32, h: f32, t: f32) -> Template {
        let glue = (0.8 * (d - t)).clamp(4.0, 15.0);
        let dust = (0.5 * w - 1.0).min(0.6 * d).max(3.0);
        let tuck = (0.3 * h).clamp(5.0, 18.0);
        let tuck_taper = (0.15 * w).min(tuck * 0.6);

        let u_l = glue;
        let u_f = u_l + (d - t);
        let u_r = u_f + w;
        let u_b = u_r + d;
        let u_end = u_b + w + t;
        let quarter = std::f32::consts::FRAC_PI_2;

        const WALLS: (f32, f32) = (0.0, 0.40);
        let mut panels = vec![
            Panel { name: "正面", outline: rect(u_f, -t, u_r, h + t), hinge: None, hidden_when_closed: false },
            Panel { name: "左側面", outline: rect(u_l, 0.0, u_f, h), hinge: Some(Hinge { parent: 0, a: [u_f, 0.0], b: [u_f, h], angle: quarter, window: WALLS }), hidden_when_closed: false },
            Panel { name: "右側面", outline: rect(u_r, 0.0, u_b, h), hinge: Some(Hinge { parent: 0, a: [u_r, 0.0], b: [u_r, h], angle: quarter, window: WALLS }), hidden_when_closed: false },
            Panel { name: "背面", outline: rect(u_b, -t, u_end, h + t), hinge: Some(Hinge { parent: 2, a: [u_b, 0.0], b: [u_b, h], angle: quarter, window: WALLS }), hidden_when_closed: false },
        ];
        let c = 2.0f32.min(h * 0.1);
        let k = glue * 0.35;
        panels.push(Panel {
            name: "のりしろ",
            outline: ccw(vec![[0.0, c + k], [u_l, c], [u_l, h - c], [0.0, h - c - k]]),
            hinge: Some(Hinge { parent: 1, a: [u_l, c], b: [u_l, h - c], angle: quarter, window: WALLS }),
            hidden_when_closed: true,
        });

        // 上と下は同じ形の鏡写し。sign = 1 が上、-1 が下。
        for (sign, names, windows) in [
            (-1.0f32, ["底の左フラップ", "底の右フラップ", "底のフタ", "底の差し込み"], [(0.40, 0.54), (0.46, 0.60), (0.54, 0.70)]),
            (1.0f32, ["上の左フラップ", "上の右フラップ", "上のフタ", "上の差し込み"], [(0.70, 0.82), (0.74, 0.88), (0.84, 1.0)]),
        ] {
            // 側面の端（上は v = h、下は v = 0）と、正面や背面の端（上は v = h + t、下は v = -t）。
            let side_edge = if sign > 0.0 { h } else { 0.0 };
            let lid_edge = if sign > 0.0 { h + t } else { -t };
            let (dust_window, tuck_window, lid_window) = (windows[0], windows[1], windows[2]);

            // 左右のフラップ。差し込みが通る正面側を、紙 1 枚ぶんと少し空ける。
            let clear = t + 0.5;
            for (name, parent, u0, u1) in [(names[0], 1usize, u_l, u_f - clear), (names[1], 2usize, u_r + clear, u_b - 0.5)] {
                let taper = (u1 - u0) * 0.15;
                panels.push(Panel {
                    name,
                    outline: ccw(vec![[u0, side_edge], [u1, side_edge], [u1 - taper, side_edge + sign * dust], [u0 + taper, side_edge + sign * dust]]),
                    hinge: Some(Hinge { parent, a: [u0, side_edge], b: [u1, side_edge], angle: quarter, window: dust_window }),
                    hidden_when_closed: true,
                });
            }

            // フタ。背面につながり、奥行きより紙 1 枚ぶん短い。
            let lid_far = lid_edge + sign * (d - t);
            let lid_index = panels.len();
            panels.push(Panel {
                name: names[2],
                outline: rect(u_b, lid_edge, u_b + w, lid_far),
                hinge: Some(Hinge { parent: 3, a: [u_b, lid_edge], b: [u_b + w, lid_edge], angle: quarter, window: lid_window }),
                hidden_when_closed: false,
            });

            // 差し込み（ベロ）。フタの先につながり、正面の内側に入る。
            let (s0, s1) = (u_b + 0.5, u_b + w - 0.5);
            panels.push(Panel {
                name: names[3],
                outline: ccw(vec![[s0, lid_far], [s1, lid_far], [s1 - tuck_taper, lid_far + sign * tuck], [s0 + tuck_taper, lid_far + sign * tuck]]),
                hinge: Some(Hinge { parent: lid_index, a: [s0, lid_far], b: [s1, lid_far], angle: quarter, window: tuck_window }),
                hidden_when_closed: true,
            });
        }

        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for p in panels.iter().flat_map(|p| p.outline.iter()) {
            min = [min[0].min(p[0]), min[1].min(p[1])];
            max = [max[0].max(p[0]), max[1].max(p[1])];
        }
        let r = w.min(d).min(h) * 0.34;
        let marks = vec![([u_r, h * 0.55], r), ([u_b + w * 0.5, h + t], r * 0.8)];
        Template { panels, thickness: t, min, max, root_center: [u_f + w * 0.5, h * 0.5], depth: d, marks }
    }

    /// 折り線の軸の向き。正の角度で子パネルが内側（+n）へ動く向きを選ぶ。
    fn hinge_axis(&self, panel: &Panel, hinge: &Hinge) -> (V3, V3) {
        let origin = [hinge.a[0], hinge.a[1], self.thickness];
        let mut dir = normalize([hinge.b[0] - hinge.a[0], hinge.b[1] - hinge.a[1], 0.0]);
        let c = centroid(&panel.outline);
        let velocity = cross(dir, sub([c[0], c[1], self.thickness], origin));
        if velocity[2] < 0.0 {
            dir = [-dir[0], -dir[1], -dir[2]];
        }
        (origin, dir)
    }

    fn hinge_angle(hinge: &Hinge, fold: f32) -> f32 {
        let (start, end) = hinge.window;
        hinge.angle * smoothstep((fold - start) / (end - start).max(1e-6))
    }

    /// 各パネルの、展開図の位置から折った位置への変換（紙の座標のまま）。
    /// `fold` は 0 が展開図、1 が組み上がった箱。
    pub fn transforms(&self, fold: f32) -> Vec<Affine> {
        let mut out: Vec<Affine> = Vec::with_capacity(self.panels.len());
        for panel in &self.panels {
            let m = match &panel.hinge {
                None => Affine::IDENTITY,
                Some(hinge) => {
                    let (origin, dir) = self.hinge_axis(panel, hinge);
                    out[hinge.parent].then_after(&Affine::rotation_about(origin, dir, Self::hinge_angle(hinge, fold)))
                }
            };
            out.push(m);
        }
        out
    }

    /// 紙の座標から表示用の座標へ。x が右、y が上、z が手前。組み上がった箱の中心が原点に来る。
    fn to_world(&self, p: V3) -> V3 {
        [p[0] - self.root_center[0], p[1] - self.root_center[1], self.depth * 0.5 + self.thickness - p[2]]
    }

    fn to_world_dir(v: V3) -> V3 {
        [v[0], v[1], -v[2]]
    }

    fn uv(&self, p: P2) -> [f32; 2] {
        [(p[0] - self.min[0]) / (self.max[0] - self.min[0]), (self.max[1] - p[1]) / (self.max[1] - self.min[1])]
    }

    /// 三角形の並び（3 頂点ずつ）で、厚みのある紙の形を作る。
    pub fn build_mesh(&self, fold: f32) -> Vec<Vertex> {
        let t = self.thickness;
        let transforms = self.transforms(fold);
        let mut out = Vec::new();
        let mut push = |m: &Affine, p: V3, normal: V3, uv: [f32; 2], kind: f32| {
            out.push(Vertex { pos: self.to_world(m.point(p)), normal: Self::to_world_dir(m.vector(normal)), uv, kind });
        };

        for (panel, m) in self.panels.iter().zip(&transforms) {
            let pts = &panel.outline;
            // 外側と内側の面。
            for i in 1..pts.len() - 1 {
                for &q in &[pts[0], pts[i], pts[i + 1]] {
                    push(m, [q[0], q[1], 0.0], [0.0, 0.0, -1.0], self.uv(q), 0.0);
                }
                for &q in &[pts[0], pts[i], pts[i + 1]] {
                    push(m, [q[0], q[1], t], [0.0, 0.0, 1.0], self.uv(q), 1.0);
                }
            }
            // 断面。
            for i in 0..pts.len() {
                let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                let normal = normalize([q[1] - p[1], p[0] - q[0], 0.0]);
                let quad = [[p[0], p[1], 0.0], [q[0], q[1], 0.0], [q[0], q[1], t], [p[0], p[1], t]];
                for idx in [0usize, 1, 2, 0, 2, 3] {
                    push(m, quad[idx], normal, self.uv(p), 2.0);
                }
            }
            // 折り線の外側を円弧で埋める。絵柄は折り線上の色をそのまま引き伸ばす。
            if let Some(hinge) = &panel.hinge {
                let angle = Self::hinge_angle(hinge, fold);
                if angle > 1e-4 {
                    let parent = &transforms[hinge.parent];
                    let (_, dir) = self.hinge_axis(panel, hinge);
                    let steps = ((angle / std::f32::consts::FRAC_PI_2) * 6.0).ceil().max(2.0) as usize;
                    let (a3, b3) = ([hinge.a[0], hinge.a[1], t], [hinge.b[0], hinge.b[1], t]);
                    let along = normalize(sub(b3, a3));
                    let ring: Vec<V3> = (0..=steps).map(|j| rotate([0.0, 0.0, -t], dir, angle * j as f32 / steps as f32)).collect();
                    for j in 0..steps {
                        let (o0, o1) = (ring[j], ring[j + 1]);
                        let (n0, n1) = (normalize(o0), normalize(o1));
                        let quad = [(add(a3, o0), n0, hinge.a), (add(b3, o0), n0, hinge.b), (add(b3, o1), n1, hinge.b), (add(a3, o1), n1, hinge.a)];
                        for idx in [0usize, 1, 2, 0, 2, 3] {
                            push(parent, quad[idx].0, quad[idx].1, self.uv(quad[idx].2), 0.0);
                        }
                        // 円弧の両端のふた。
                        for (center, sign, uv_at) in [(a3, -1.0f32, hinge.a), (b3, 1.0f32, hinge.b)] {
                            let normal = [along[0] * sign, along[1] * sign, along[2] * sign];
                            for p in [center, add(center, o0), add(center, o1)] {
                                push(parent, p, normal, self.uv(uv_at), 2.0);
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

/// 頂点全体を包む球（中心と半径）。カメラを合わせるのに使う。
pub fn bounding_sphere(vertices: &[Vertex]) -> (V3, f32) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in vertices {
        for i in 0..3 {
            min[i] = min[i].min(v.pos[i]);
            max[i] = max[i].max(v.pos[i]);
        }
    }
    let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5, (min[2] + max[2]) * 0.5];
    let half = sub(max, center);
    (center, dot(half, half).sqrt().max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 120.0;
    const D: f32 = 60.0;
    const H: f32 = 160.0;
    const T: f32 = 3.0;

    fn close(a: V3, b: V3) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-2, "{a:?} と {b:?} が一致しない");
        }
    }

    fn index(tpl: &Template, name: &str) -> usize {
        tpl.panels.iter().position(|p| p.name == name).unwrap()
    }

    // 展開図上の位置（型の作り方と同じ式）。
    fn layout() -> (f32, f32, f32, f32) {
        let glue = (0.8 * (D - T)).clamp(4.0, 15.0);
        let u_f = glue + (D - T);
        (glue, u_f, u_f + W, u_f + W + D)
    }

    #[test]
    fn flat_sheet_is_untouched() {
        let tpl = Template::tuck_box(W, D, H, T);
        for m in tpl.transforms(0.0) {
            close(m.point([12.0, 34.0, 1.0]), [12.0, 34.0, 1.0]);
        }
    }

    #[test]
    fn outlines_are_counter_clockwise() {
        let tpl = Template::tuck_box(W, D, H, T);
        for p in &tpl.panels {
            assert!(signed_area(&p.outline) > 0.0, "{} が時計回り", p.name);
        }
    }

    #[test]
    fn walls_close_into_a_box() {
        let tpl = Template::tuck_box(W, D, H, T);
        let m = tpl.transforms(1.0);
        let (u_l, u_f, u_r, u_b) = layout();

        // 右側面の内側は、正面の右端から奥へまっすぐ伸びる。
        close(m[index(&tpl, "右側面")].point([u_r + D, 50.0, T]), [u_r, 50.0, T + D]);
        // 左側面も同じ。
        close(m[index(&tpl, "左側面")].point([u_f - 20.0, 50.0, T]), [u_f, 50.0, T + 20.0]);
        // 背面の内側は、正面の内側から奥行きぶん離れる。端は左側面の外側とそろう。
        close(m[index(&tpl, "背面")].point([u_b + W + T, 50.0, T]), [u_f - T, 50.0, T + D]);
        // のりしろの外側は、背面の内側にぴったり付く。
        close(m[index(&tpl, "のりしろ")].point([u_l - 5.0, 50.0, 0.0]), [u_f + 5.0, 50.0, T + D]);
    }

    #[test]
    fn lid_sits_on_flaps_and_tuck_goes_inside_front() {
        let tpl = Template::tuck_box(W, D, H, T);
        let m = tpl.transforms(1.0);
        let (u_l, _u_f, u_r, u_b) = layout();

        // 上の左フラップの内側は、側面の上端の高さで水平になる。
        let flap = m[index(&tpl, "上の左フラップ")].point([u_l + 10.0, H + 8.0, T]);
        assert!((flap[1] - H).abs() < 1e-2, "フラップの高さ {}", flap[1]);
        // フタの内側は、それより紙 1 枚ぶん上。
        close(m[index(&tpl, "上のフタ")].point([u_b + 30.0, H + T + 10.0, T]), [u_r - 30.0, H + T, T + D - 10.0]);
        // 差し込みの外側は、正面の内側に付く。
        let tuck = m[index(&tpl, "上の差し込み")].point([u_b + 30.0, H + D + 6.0, 0.0]);
        close(tuck, [u_r - 30.0, H + T - 6.0, T]);
        // 底も同じ。
        let bottom = m[index(&tpl, "底の差し込み")].point([u_b + 30.0, -D - 6.0, 0.0]);
        close(bottom, [u_r - 30.0, -T + 6.0, T]);
    }

    #[test]
    fn mesh_is_a_triangle_list() {
        let tpl = Template::tuck_box(W, D, H, T);
        for fold in [0.0, 0.3, 1.0] {
            let mesh = tpl.build_mesh(fold);
            assert!(!mesh.is_empty() && mesh.len() % 3 == 0);
            assert!(mesh.iter().all(|v| v.pos.iter().all(|c| c.is_finite())));
        }
    }
}
