//! 型紙（パネルと折り線）と、厚みのある紙を折る・曲げる計算。
//!
//! 座標は「紙の座標」(u, v, n) で持つ。u と v は展開図上の位置（mm）、n は紙の厚み方向で、
//! n = 0 が外側（印刷面）、n = 厚み が内側。
//!
//! - 谷折りは内側どうしが近づく向きで、軸を内側の面に置く。山折りは逆で、軸を外側の面に置く。
//!   折り線にできるすき間は、円弧の面で埋める。
//! - パネルは、親との折り線に平行な線を保ったまま、一定の曲がり具合で丸めることができる（円筒の一部になる）。
//!   紙は伸び縮みしないので、展開図上の位置と立体上の位置は一対一のまま。

use bytemuck::{Pod, Zeroable};

use crate::math::{Affine, V3, add, cross, dot, normalize, rotate, scale, sub};

pub type P2 = [f32; 2];

/// 親パネルとのつながり（折り線）。
pub struct Hinge {
    pub parent: usize,
    /// 折り線の両端（展開図上）。
    pub a: P2,
    pub b: P2,
    /// 折りきったときの角度（ラジアン）。正が谷折り、負が山折り。
    pub angle: f32,
    /// 折りが進む区間。組み立ての進み具合 0〜1 のうち、どこからどこまでで折るか。
    pub window: (f32, f32),
    /// お客さんが開け閉めするときの動き。
    pub motion: Motion,
}

/// お客さんが開け閉めするときの、折り線の動き方。
///
/// 開け閉めは「主役の角度」1 つで決まる（ゲーブル型なら屋根の傾き）。ほかの折り線は、それに連動して決まる。
/// `Fixed` の折り線は、製造時に折ってのりや差し込みで固定したまま、開け閉めでは動かない。
#[derive(Clone, Copy)]
pub enum Motion {
    Fixed,
    /// 主役の角度に係数を掛けたぶん折れる。負なら山折り。
    Follow(f32),
    /// ゲーブル型の、側面の上の三角形。値は「側面の幅の半分 ÷ 屋根の斜面の長さ」。
    GableCenter(f32),
    /// ゲーブル型の、屋根の裏へ折り込む三角形。組み立てで折った 90 度から、さらに折れるぶんを返す。
    GableWing(f32),
}

/// ゲーブル型の角では、屋根、折り込む三角形、側面の三角形の 3 枚が 1 点を中心につながって動く。
/// 屋根が `theta` 傾いたとき、側面の三角形が内側へ倒れる角度。
/// 折り込む三角形の頂角が変わらない、という条件から求まる。
fn gable_center_angle(theta: f32, k: f32) -> f32 {
    ((1.0 - k * theta.sin()) / theta.cos()).clamp(-1.0, 1.0).acos()
}

impl Motion {
    fn angle(self, theta: f32) -> f32 {
        match self {
            Motion::Fixed => 0.0,
            Motion::Follow(scale) => scale * theta,
            Motion::GableCenter(k) => gable_center_angle(theta, k),
            Motion::GableWing(k) => (-gable_center_angle(theta, k).sin() / k).clamp(-1.0, 1.0).acos() - std::f32::consts::FRAC_PI_2,
        }
    }
}

impl Hinge {
    fn is_mountain(&self) -> bool {
        self.angle < 0.0 || matches!(self.motion, Motion::Follow(scale) if scale < 0.0)
    }
}

/// パネルの丸め。折り線に平行な線を保ったまま、折り線から離れるほど巻いていく。
pub struct Bend {
    /// 丸めきったときの曲がり具合（半径の逆数、1/mm）。正が内側へ巻く向き。
    pub curvature: f32,
    pub window: (f32, f32),
    /// 巻きはじめの線（展開図上の 2 点）。None なら、親との折り線から巻きはじめる。
    /// 根のパネルのように、折り線を持たないパネルを丸めるときに使う。
    pub line: Option<(P2, P2)>,
}

/// 曲線の折り。親パネルの端にある曲線の折り線で、親の丸みに連動して折れる。
///
/// 親が円筒に丸まると、折り線は立体の中で 1 枚の平面に乗る曲線になる。
/// 折った先の面は、親の面をそのまま延ばしたものを、その平面で鏡に映した形になる（これも伸び縮みしない曲げ方）。
pub struct CurvedFold {
    pub parent: usize,
    /// 折り線上の 3 点（両端と中央）。この 3 点が乗る平面で映す。
    pub crease: [P2; 3],
    /// 折り線の形（折れ線で近似したもの）。型紙のガイド線に使う。
    pub line: Vec<P2>,
}

/// 紙の一区画。輪郭は凸多角形で、反時計回り。
pub struct Panel {
    pub name: &'static str,
    pub outline: Vec<P2>,
    pub hinge: Option<Hinge>,
    pub bend: Option<Bend>,
    /// 曲線の折りで親につながるパネル。
    pub curved: Option<CurvedFold>,
    /// 面を張るときに、巻く向きに必ずこの数だけ細片に分ける。0 なら、丸めたときだけ必要なぶん分ける。
    /// 輪郭がへこんでいるパネル（曲線の折り線で切り欠かれた面）で、細片を凸に保つために使う。
    pub slices: usize,
    /// 組み上がると隠れる部分（のりしろ、差し込みなど）か。絵柄の色分けに使う。
    pub hidden_when_closed: bool,
}

impl Panel {
    fn flat(name: &'static str, outline: Vec<P2>, hinge: Option<Hinge>, hidden_when_closed: bool) -> Panel {
        Panel { name, outline, hinge, bend: None, curved: None, slices: 0, hidden_when_closed }
    }
}

/// 型紙。パネルは親が子より先に並ぶ。
pub struct Template {
    pub panels: Vec<Panel>,
    pub thickness: f32,
    pub min: P2,
    pub max: P2,
    /// 紙の座標から表示用の座標への変換（x が右、y が上、z が手前）。印刷面が外を向くよう、裏返しを含む。
    /// 展開図のときは、印刷面が手前を向く。
    world: Affine,
    /// 組み上がるにつれて全体を手前へ倒す角度（ラジアン）と、倒す区間。
    tilt: f32,
    tilt_window: (f32, f32),
    /// 確認用の絵柄で置く円（中心と半径）。
    pub marks: Vec<(P2, f32)>,
    /// 抜き穴（中心 u, v と、半分の幅と高さ）。角の丸い四角として扱う。
    pub holes: Vec<[f32; 4]>,
    /// お客さんが開け閉めするときの、主役の角度の最大値（閉じた状態。ラジアン）。0 なら開け閉めの動きを持たない。
    pub open_angle: f32,
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

/// ある開き具合での、パネル 1 枚の置かれ方。
pub struct Placement {
    /// 平らなままのときの変換（親の位置 × 折り線まわりの回転）。
    base: Affine,
    bend: Option<BendNow>,
    /// 曲線の折りで映す変換（後から掛ける鏡映と、先に掛ける厚み方向の裏返し）。
    /// 鏡映を 2 回重ねるので、全体としては裏返しのない回転と移動になる。
    mirror: Option<(Affine, Affine)>,
}

#[derive(Clone, Copy)]
struct BendNow {
    /// 折り線上の点（内側の面）。
    origin: V3,
    /// 折り線から離れる向き（展開図上、パネルの内側へ）。
    s_dir: V3,
    /// 巻く軸の向き（折り線と平行）。
    axis: V3,
    kappa: f32,
}

/// 曲面を平らな細片に分けるときの、1 片あたりの角度（約 4 度）。
const STRIP_ANGLE: f32 = 0.07;

impl Placement {
    fn active_bend(&self) -> Option<&BendNow> {
        self.bend.as_ref().filter(|b| b.kappa.abs() > 1e-6)
    }

    /// 巻きはじめの線からの距離。丸めの設定を持たないパネルでは None。
    fn distance(&self, p: P2) -> Option<f32> {
        self.bend.as_ref().map(|b| dot(sub([p[0], p[1], b.origin[2]], b.origin), b.s_dir))
    }

    /// 展開図上の点 `p` の近くを、そのまま立体へ運ぶ変換。
    /// 丸めたパネルでは場所ごとに違う（折り線に平行な線ごとに、巻いたぶん回っている）。
    pub fn at(&self, p: P2) -> Affine {
        let core = self.at_unmirrored(p);
        match &self.mirror {
            Some((after, before)) => after.then_after(&core.then_after(before)),
            None => core,
        }
    }

    fn at_unmirrored(&self, p: P2) -> Affine {
        let Some(b) = self.active_bend() else { return self.base };
        let s = dot(sub([p[0], p[1], b.origin[2]], b.origin), b.s_dir);
        if s.abs() < 1e-6 {
            return self.base;
        }
        // 巻きの中心は、内側の面から半径ぶん内側。そこを軸に回してから、折り線の位置まで戻す。
        let pivot = add(add(b.origin, scale(b.s_dir, s)), [0.0, 0.0, 1.0 / b.kappa]);
        let mut frame = Affine::rotation_about(pivot, b.axis, b.kappa * s);
        frame.t = sub(frame.t, scale(b.s_dir, s));
        self.base.then_after(&frame)
    }

    /// 紙の座標の点を、折った位置へ（紙の座標のまま）。
    pub fn point(&self, p: V3) -> V3 {
        self.at([p[0], p[1]]).point(p)
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn progress(window: (f32, f32), fold: f32) -> f32 {
    smoothstep((fold - window.0) / (window.1 - window.0).max(1e-6))
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

fn bounds(panels: &[Panel]) -> (P2, P2) {
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for p in panels.iter().flat_map(|p| p.outline.iter()) {
        min = [min[0].min(p[0]), min[1].min(p[1])];
        max = [max[0].max(p[0]), max[1].max(p[1])];
    }
    (min, max)
}

fn rect(u0: f32, v0: f32, u1: f32, v1: f32) -> Vec<P2> {
    ccw(vec![[u0, v0], [u1, v0], [u1, v1], [u0, v1]])
}

fn lerp(p: P2, q: P2, x: f32) -> P2 {
    [p[0] + (q[0] - p[0]) * x, p[1] + (q[1] - p[1]) * x]
}

/// 凸多角形を、`value(p) >= 0` の側だけ残して切る。
fn clip(poly: &[P2], value: impl Fn(P2) -> f32) -> Vec<P2> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let (vp, vq) = (value(p), value(q));
        if vp >= 0.0 {
            out.push(p);
        }
        if (vp >= 0.0) != (vq >= 0.0) {
            out.push(lerp(p, q, vp / (vp - vq)));
        }
    }
    out
}

/// 正面が手前を向く箱の、表示用の座標への変換。
fn upright_world(center: P2, depth: f32, t: f32) -> Affine {
    Affine { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]], t: [-center[0], -center[1], depth * 0.5 + t] }
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
            Panel::flat("正面", rect(u_f, -t, u_r, h + t), None, false),
            Panel::flat("左側面", rect(u_l, 0.0, u_f, h), Some(Hinge { parent: 0, a: [u_f, 0.0], b: [u_f, h], angle: quarter, window: WALLS, motion: Motion::Fixed }), false),
            Panel::flat("右側面", rect(u_r, 0.0, u_b, h), Some(Hinge { parent: 0, a: [u_r, 0.0], b: [u_r, h], angle: quarter, window: WALLS, motion: Motion::Fixed }), false),
            Panel::flat("背面", rect(u_b, -t, u_end, h + t), Some(Hinge { parent: 2, a: [u_b, 0.0], b: [u_b, h], angle: quarter, window: WALLS, motion: Motion::Fixed }), false),
        ];
        let c = 2.0f32.min(h * 0.1);
        let k = glue * 0.35;
        panels.push(Panel::flat(
            "のりしろ",
            ccw(vec![[0.0, c + k], [u_l, c], [u_l, h - c], [0.0, h - c - k]]),
            Some(Hinge { parent: 1, a: [u_l, c], b: [u_l, h - c], angle: quarter, window: WALLS, motion: Motion::Fixed }),
            true,
        ));

        // 上と下は同じ形の鏡写し。sign = 1 が上、-1 が下。
        for (sign, names, windows) in [
            (-1.0f32, ["底の左フラップ", "底の右フラップ", "底のフタ", "底の差し込み"], [(0.40, 0.54), (0.46, 0.60), (0.54, 0.70)]),
            (1.0f32, ["上の左フラップ", "上の右フラップ", "上のフタ", "上の差し込み"], [(0.70, 0.82), (0.74, 0.88), (0.84, 1.0)]),
        ] {
            let side_edge = if sign > 0.0 { h } else { 0.0 };
            let lid_edge = if sign > 0.0 { h + t } else { -t };
            let (dust_window, tuck_window, lid_window) = (windows[0], windows[1], windows[2]);

            // 左右のフラップ。差し込みが通る正面側を、紙 1 枚ぶんと少し空ける。
            let clear = t + 0.5;
            for (name, parent, u0, u1) in [(names[0], 1usize, u_l, u_f - clear), (names[1], 2usize, u_r + clear, u_b - 0.5)] {
                let taper = (u1 - u0) * 0.15;
                panels.push(Panel::flat(
                    name,
                    ccw(vec![[u0, side_edge], [u1, side_edge], [u1 - taper, side_edge + sign * dust], [u0 + taper, side_edge + sign * dust]]),
                    Some(Hinge { parent, a: [u0, side_edge], b: [u1, side_edge], angle: quarter, window: dust_window, motion: Motion::Fixed }),
                    true,
                ));
            }

            // フタ。背面につながり、奥行きより紙 1 枚ぶん短い。
            let lid_far = lid_edge + sign * (d - t);
            let lid_index = panels.len();
            panels.push(Panel::flat(names[2], rect(u_b, lid_edge, u_b + w, lid_far), Some(Hinge { parent: 3, a: [u_b, lid_edge], b: [u_b + w, lid_edge], angle: quarter, window: lid_window, motion: Motion::Fixed }), false));

            // 差し込み（ベロ）。フタの先につながり、正面の内側に入る。
            let (s0, s1) = (u_b + 0.5, u_b + w - 0.5);
            panels.push(Panel::flat(
                names[3],
                ccw(vec![[s0, lid_far], [s1, lid_far], [s1 - tuck_taper, lid_far + sign * tuck], [s0 + tuck_taper, lid_far + sign * tuck]]),
                Some(Hinge { parent: lid_index, a: [s0, lid_far], b: [s1, lid_far], angle: quarter, window: tuck_window, motion: Motion::Fixed }),
                true,
            ));
        }

        let (min, max) = bounds(&panels);
        let r = w.min(d).min(h) * 0.34;
        let marks = vec![([u_r, h * 0.55], r), ([u_b + w * 0.5, h + t], r * 0.8)];
        Template { panels, thickness: t, min, max, world: upright_world([u_f + w * 0.5, h * 0.5], d, t), tilt: 0.0, tilt_window: (0.0, 0.5), marks, holes: Vec::new(), open_angle: 0.0 }
    }

    /// 正多角柱（三角柱、五角柱など）。`side` は内側の一辺の長さ、`h` は内寸の高さ（mm）。
    ///
    /// - 側面は外角（360 度 / n）ずつ折る。
    /// - のりしろは最初の側面の内側に入る。そのぶん最後の側面を短く、最初の側面を長くする。
    ///   ずらす量は「厚み / sin(外角)」で、直角のときは厚みと同じになる。
    /// - フタは多角形で、ほかの側面のフラップの上に乗る。そのぶんフタのつく側面を上下に 1 枚ぶん高くする。
    pub fn prism(n: usize, side: f32, h: f32, t: f32) -> Template {
        let n = n.clamp(3, 12);
        let exterior = std::f32::consts::TAU / n as f32;
        let interior = std::f32::consts::PI - exterior;
        let quarter = std::f32::consts::FRAC_PI_2;
        let shift = t / exterior.sin();
        let glue = (0.3 * side).clamp(4.0, 12.0);
        let flap = 0.2 * side * (interior * 0.5).tan();
        let inset = 0.2 * side;

        let u0 = shift;
        let wall_start = |k: usize| u0 + k as f32 * side;
        let last_end = wall_start(n) - shift;
        const WALLS: (f32, f32) = (0.0, 0.50);

        let mut panels = vec![Panel::flat("側面 1", rect(u0 - shift, -t, u0 + side, h + t), None, false)];
        const NAMES: [&str; 12] = ["側面 1", "側面 2", "側面 3", "側面 4", "側面 5", "側面 6", "側面 7", "側面 8", "側面 9", "側面 10", "側面 11", "側面 12"];
        for k in 1..n {
            let end = if k == n - 1 { last_end } else { wall_start(k + 1) };
            panels.push(Panel::flat(NAMES[k], rect(wall_start(k), 0.0, end, h), Some(Hinge { parent: k - 1, a: [wall_start(k), 0.0], b: [wall_start(k), h], angle: exterior, window: WALLS, motion: Motion::Fixed }), false));
        }
        let c = 2.0f32.min(h * 0.1);
        let k = glue * 0.35;
        panels.push(Panel::flat(
            "のりしろ",
            ccw(vec![[last_end, c], [last_end + glue, c + k], [last_end + glue, h - c - k], [last_end, h - c]]),
            Some(Hinge { parent: n - 1, a: [last_end, c], b: [last_end, h - c], angle: exterior, window: WALLS, motion: Motion::Fixed }),
            true,
        ));

        for (sign, flap_name, lid_name, flap_window, lid_window) in [(-1.0f32, "底のフラップ", "底のフタ", (0.50, 0.62), (0.60, 0.74)), (1.0f32, "上のフラップ", "上のフタ", (0.74, 0.86), (0.86, 1.0))] {
            let side_edge = if sign > 0.0 { h } else { 0.0 };
            let lid_edge = if sign > 0.0 { h + t } else { -t };
            for wall in 1..n {
                let a = wall_start(wall) + 0.5;
                let b = (if wall == n - 1 { last_end } else { wall_start(wall + 1) }) - 0.5;
                panels.push(Panel::flat(
                    flap_name,
                    ccw(vec![[a, side_edge], [b, side_edge], [b - inset, side_edge + sign * flap], [a + inset, side_edge + sign * flap]]),
                    Some(Hinge { parent: wall, a: [a, side_edge], b: [b, side_edge], angle: quarter, window: flap_window, motion: Motion::Fixed }),
                    true,
                ));
            }
            // フタは、最初の側面の端を一辺とする正多角形。
            let mut lid = vec![[u0, lid_edge], [u0 + side, lid_edge]];
            for i in 1..n - 1 {
                let prev = lid[lid.len() - 1];
                let angle = exterior * i as f32;
                lid.push([prev[0] + side * angle.cos(), prev[1] + sign * side * angle.sin()]);
            }
            panels.push(Panel::flat(lid_name, ccw(lid), Some(Hinge { parent: 0, a: [u0, lid_edge], b: [u0 + side, lid_edge], angle: quarter, window: lid_window, motion: Motion::Fixed }), false));
        }

        let (min, max) = bounds(&panels);
        let r = side.min(h) * 0.3;
        let marks = vec![([u0 + side, h * 0.55], r), ([u0 + side * 0.5, h + t], r * 0.8)];
        let depth = side / (std::f32::consts::PI / n as f32).tan();
        Template { panels, thickness: t, min, max, world: upright_world([u0 + side * 0.5, h * 0.5], depth, t), tilt: 0.0, tilt_window: (0.0, 0.5), marks, holes: Vec::new(), open_angle: 0.0 }
    }

    /// 手提げ型。底は平らな長方形で、正面と背面が丸みを持って立ち上がり、上で合わさって持ち手になる。
    ///
    /// `w` と `d` は底の内寸、`l` は正面と背面の長さ（底の折り線から、上で合わさる線まで。mm）。
    /// `bulge` はふくらみで、正面が底から上までに向きを変える角度（ラジアン）。0 なら平らな三角屋根になる。
    ///
    /// 正面と背面は、底との折り線に平行な線を保ったまま丸める（円筒の一部）。
    /// 立ち上がる角度は、上の端がちょうど中央で合わさるように、ふくらみから逆算する。
    /// 左右は、その断面と同じ形の平らな側面でふさぐ。
    pub fn carry_bag(w: f32, d: f32, l: f32, t: f32, bulge: f32) -> Template {
        const SIDES: (f32, f32) = (0.0, 0.35);
        const MAIN: (f32, f32) = (0.25, 1.0);
        let l = l.max(d * 0.5 + 5.0);
        let psi = bulge.clamp(0.0, 1.5);
        let kappa = psi / l;
        // 内側の面の断面で考える。奥行き方向を y（中央が 0、正面の底が -d/2）、高さを z とする。
        // 正面は底から角度 theta0（内向きの水平から測る）で立ち上がり、長さ l の間に psi だけ内側へ向きを変える。
        // 持ち手は外側の面を軸に折り返すので、内側の面の上端は中央より紙の厚みの一部だけ手前で止める。
        let mut gap = 0.0f32;
        let (mut theta0, mut theta_end) = (0.0f32, 0.0f32);
        for _ in 0..4 {
            let half = (d * 0.5 - gap).max(1.0);
            let chord_cos = if psi < 1e-4 { half / l } else { half * kappa / (2.0 * (psi * 0.5).sin()) };
            theta0 = psi * 0.5 + chord_cos.clamp(-1.0, 1.0).acos();
            theta_end = theta0 - psi;
            gap = t * (1.0 - theta_end.sin());
        }
        let rise = std::f32::consts::PI - theta0;
        let handle_fold = -(std::f32::consts::FRAC_PI_2 - theta_end);
        let bend = || if kappa > 1e-6 { Some(Bend { curvature: kappa, window: MAIN, line: None }) } else { None };

        let c = w * 0.5;
        let half_handle = w * 0.28;
        let handle_height = (0.3 * l).clamp(30.0, 60.0);
        let top = d + l;

        let mut panels = vec![Panel::flat("底", rect(0.0, 0.0, w, d), None, false)];
        panels.push(Panel { name: "正面", outline: rect(0.0, d, w, top), hinge: Some(Hinge { parent: 0, a: [0.0, d], b: [w, d], angle: rise, window: MAIN, motion: Motion::Fixed }), bend: bend(), curved: None, slices: 0, hidden_when_closed: false });
        panels.push(Panel { name: "背面", outline: rect(0.0, -l, w, 0.0), hinge: Some(Hinge { parent: 0, a: [0.0, 0.0], b: [w, 0.0], angle: rise, window: MAIN, motion: Motion::Fixed }), bend: bend(), curved: None, slices: 0, hidden_when_closed: false });
        for (name, parent, edge, sign) in [("正面の持ち手", 1usize, top, 1.0f32), ("背面の持ち手", 2usize, -l, -1.0f32)] {
            panels.push(Panel::flat(
                name,
                ccw(vec![[c - half_handle, edge], [c + half_handle, edge], [c + half_handle * 0.82, edge + sign * handle_height], [c - half_handle * 0.82, edge + sign * handle_height]]),
                Some(Hinge { parent, a: [c - half_handle, edge], b: [c + half_handle, edge], angle: handle_fold, window: MAIN, motion: Motion::Fixed }),
                false,
            ));
        }

        // 側面の輪郭は、正面と背面の外側の面がたどる断面と同じ形。
        let steps = 24;
        let profile = |i: usize| -> (f32, f32) {
            let s = l * i as f32 / steps as f32;
            let theta = theta0 - kappa * s;
            let (y, z) = if kappa > 1e-6 { (-d * 0.5 + (theta0.sin() - theta.sin()) / kappa, (theta.cos() - theta0.cos()) / kappa) } else { (-d * 0.5 + s * theta0.cos(), s * theta0.sin()) };
            (y - t * theta.sin(), (z + t * theta.cos()).max(0.0))
        };
        for (name, edge, sign) in [("右の側面", w, 1.0f32), ("左の側面", 0.0f32, -1.0f32)] {
            let mut outline = vec![[edge, 0.0], [edge, d]];
            outline.extend((0..=steps).map(|i| profile(i)).map(|(y, z)| [edge + sign * z, d * 0.5 - y]));
            outline.extend((0..=steps).rev().map(|i| profile(i)).map(|(y, z)| [edge + sign * z, d * 0.5 + y]));
            panels.push(Panel::flat(name, ccw(outline), Some(Hinge { parent: 0, a: [edge, 0.0], b: [edge, d], angle: std::f32::consts::FRAC_PI_2, window: SIDES, motion: Motion::Fixed }), false));
        }

        let (min, max) = bounds(&panels);
        let r = w.min(l) * 0.22;
        let marks = vec![([c, d + l * 0.5], r), ([w, d + l * 0.3], r * 0.6)];
        let hole = |edge: f32, sign: f32| [c, edge + sign * handle_height * 0.55, half_handle * 0.5, handle_height * 0.2];
        // 展開図では印刷面を手前に向け、組み上がるにつれて 90 度倒して、底を水平に寝かせる。
        // 倒しきると、x が幅、y が高さ（底の内側が 0）、z が奥行き（正面が手前）になる。
        let world = upright_world([c, d * 0.5], 0.0, t);
        Template { panels, thickness: t, min, max, world, tilt: std::f32::consts::FRAC_PI_2, tilt_window: (0.0, 0.5), marks, holes: vec![hole(top, 1.0), hole(-l, -1.0)], open_angle: 0.0 }
    }

    /// ゲーブル型（屋根の形に閉じる手提げ箱）。寸法は内寸（mm）で、`roof` は胴の上端から屋根のてっぺんまでの高さ。
    ///
    /// 胴と底は差し込み箱と同じ作りで、製造時に組んで固定する。お客さんが動かすのは屋根だけ。
    /// - 正面と背面の屋根が内側へ傾き、上で合わさる。その先が持ち手になる。
    /// - 側面の上は、中央の三角形が内側へ倒れ、両脇の三角形が屋根の裏へ折り込まれる。
    /// - 屋根の傾きを決めると、ほかの折り線の角度はすべて決まる（`Motion`）。
    pub fn gable(w: f32, d: f32, h: f32, roof: f32, t: f32) -> Template {
        // 側面の三角形が中央でぶつからないよう、奥行きは幅までにする。
        let d = d.min(w);
        let roof = roof.max(5.0);
        let glue = (0.8 * (d - t)).clamp(4.0, 15.0);
        let dust = (0.5 * w - 1.0).min(0.6 * d).max(3.0);
        let tuck = (0.3 * h).clamp(5.0, 18.0);
        let tuck_taper = (0.15 * w).min(tuck * 0.6);
        let slope = ((d * 0.5).powi(2) + roof * roof).sqrt();
        let closed = (d * 0.5 / slope).asin();
        let handle = (0.25 * h).clamp(22.0, 40.0);

        let u_l = glue;
        let u_f = u_l + (d - t);
        let u_r = u_f + w;
        let u_b = u_r + d;
        let u_end = u_b + w + t;
        let quarter = std::f32::consts::FRAC_PI_2;
        let fixed = Motion::Fixed;

        const WALLS: (f32, f32) = (0.0, 0.50);
        let mut panels = vec![
            Panel::flat("正面", rect(u_f, -t, u_r, h), None, false),
            Panel::flat("左側面", rect(u_l, 0.0, u_f, h), Some(Hinge { parent: 0, a: [u_f, 0.0], b: [u_f, h], angle: quarter, window: WALLS, motion: fixed }), false),
            Panel::flat("右側面", rect(u_r, 0.0, u_b, h), Some(Hinge { parent: 0, a: [u_r, 0.0], b: [u_r, h], angle: quarter, window: WALLS, motion: fixed }), false),
            Panel::flat("背面", rect(u_b, -t, u_end, h), Some(Hinge { parent: 2, a: [u_b, 0.0], b: [u_b, h], angle: quarter, window: WALLS, motion: fixed }), false),
        ];
        let c = 2.0f32.min(h * 0.1);
        let k = glue * 0.35;
        panels.push(Panel::flat(
            "のりしろ",
            ccw(vec![[0.0, c + k], [u_l, c], [u_l, h - c], [0.0, h - c - k]]),
            Some(Hinge { parent: 1, a: [u_l, c], b: [u_l, h - c], angle: quarter, window: WALLS, motion: fixed }),
            true,
        ));

        // 底は差し込み箱と同じ。
        let clear = t + 0.5;
        for (name, parent, u0, u1) in [("底の左フラップ", 1usize, u_l, u_f - clear), ("底の右フラップ", 2usize, u_r + clear, u_b - 0.5)] {
            let taper = (u1 - u0) * 0.15;
            panels.push(Panel::flat(
                name,
                ccw(vec![[u0, 0.0], [u1, 0.0], [u1 - taper, -dust], [u0 + taper, -dust]]),
                Some(Hinge { parent, a: [u0, 0.0], b: [u1, 0.0], angle: quarter, window: (0.50, 0.70), motion: fixed }),
                true,
            ));
        }
        let lid_far = -t - (d - t);
        let bottom_lid = panels.len();
        panels.push(Panel::flat("底のフタ", rect(u_b, -t, u_b + w, lid_far), Some(Hinge { parent: 3, a: [u_b, -t], b: [u_b + w, -t], angle: quarter, window: (0.75, 1.0), motion: fixed }), false));
        let (s0, s1) = (u_b + 0.5, u_b + w - 0.5);
        panels.push(Panel::flat(
            "底の差し込み",
            ccw(vec![[s0, lid_far], [s1, lid_far], [s1 - tuck_taper, lid_far - tuck], [s0 + tuck_taper, lid_far - tuck]]),
            Some(Hinge { parent: bottom_lid, a: [s0, lid_far], b: [s1, lid_far], angle: quarter, window: (0.60, 0.80), motion: fixed }),
            true,
        ));

        // 屋根と持ち手。組み立てでは折らず、開け閉めで動く。
        let ridge = h + slope;
        let mut roofs = [0usize; 2];
        for (i, (roof_name, handle_name, wall, u0)) in [("正面の屋根", "正面の持ち手", 0usize, u_f), ("背面の屋根", "背面の持ち手", 3usize, u_b)].into_iter().enumerate() {
            roofs[i] = panels.len();
            panels.push(Panel::flat(roof_name, rect(u0, h, u0 + w, ridge), Some(Hinge { parent: wall, a: [u0, h], b: [u0 + w, h], angle: 0.0, window: WALLS, motion: Motion::Follow(1.0) }), false));
            panels.push(Panel::flat(
                handle_name,
                rect(u0, ridge, u0 + w, ridge + handle),
                Some(Hinge { parent: roofs[i], a: [u0, ridge], b: [u0 + w, ridge], angle: 0.0, window: WALLS, motion: Motion::Follow(-1.0) }),
                false,
            ));
        }

        // 側面の上。中央の三角形は側面に、両脇の三角形は屋根につながる。
        // 両脇の三角形は、胴を筒にするときに屋根に対して 90 度折れ、閉じるときにさらに折れて屋根の裏に付く。
        // 左側は、のりしろのぶん側面が紙 1 枚ぶん短いので、三角形もそのぶん小さい。
        for (center_name, wing_name, wall, base0, base1, front_edge, back_edge, front_dir, back_dir) in [
            ("右の三角", "右の折り込み", 2usize, u_r, u_b, u_r, u_b, 1.0f32, -1.0f32),
            ("左の三角", "左の折り込み", 1usize, u_l, u_f, u_f, u_b + w, -1.0f32, 1.0f32),
        ] {
            let half = (base1 - base0) * 0.5;
            let ratio = half / slope;
            panels.push(Panel::flat(
                center_name,
                ccw(vec![[base0, h], [base1, h], [base0 + half, ridge]]),
                Some(Hinge { parent: wall, a: [base0, h], b: [base1, h], angle: 0.0, window: WALLS, motion: Motion::GableCenter(ratio) }),
                false,
            ));
            for (roof_index, edge, dir) in [(roofs[0], front_edge, front_dir), (roofs[1], back_edge, back_dir)] {
                panels.push(Panel::flat(
                    wing_name,
                    ccw(vec![[edge, h], [edge, ridge], [edge + dir * half, ridge]]),
                    Some(Hinge { parent: roof_index, a: [edge, h], b: [edge, ridge], angle: quarter, window: WALLS, motion: Motion::GableWing(ratio) }),
                    true,
                ));
            }
        }

        let (min, max) = bounds(&panels);
        let r = w.min(h) * 0.26;
        let marks = vec![([u_f + w * 0.5, h * 0.5], r), ([u_f + w * 0.22, h], r * 0.55)];
        let hole = |u0: f32| [u0 + w * 0.5, ridge + handle * 0.5, w * 0.14, handle * 0.24];
        Template {
            panels,
            thickness: t,
            min,
            max,
            world: upright_world([u_f + w * 0.5, (h + roof) * 0.5], d, t),
            tilt: 0.0, tilt_window: (0.0, 0.5),
            marks,
            holes: vec![hole(u_f), hole(u_b)],
            open_angle: closed,
        }
    }

    /// 持ち手つきピロー型。2 枚の面が下の折り線でつながり、丸くふくらんで上で合わさり、その先が持ち手になる。
    /// 左右の端は、曲線の折り線でフラップを内側へ折り込んで閉じる。
    ///
    /// `len` は横の長さ、`height` は 1 枚の面の高さ（丸みに沿って測る。mm）。
    /// `bulge` はふくらみで、1 枚の面が下から上までに向きを変える角度（ラジアン）。
    ///
    /// 曲線の折り線の形は、ふくらみから自動で決まる。面を丸めたときに、折り線が 45 度に傾いた平面に乗る形にしてあり、
    /// フラップはその平面で映した位置（奥行き方向へまっすぐ倒れ込んだ形）に来る。
    pub fn pillow(len: f32, height: f32, t: f32, bulge: f32) -> Template {
        const MAIN: (f32, f32) = (0.0, 1.0);
        const SLICES: usize = 24;
        let psi = bulge.clamp(0.2, 1.6);
        let kappa = psi / height;
        let half_turn = psi * 0.5;
        // 面の上のある高さ v での、ふくらみの深さ（2 枚が合わさる平面からの距離）。端で 0、中央で最大。
        let depth = |v: f32| ((kappa * (v - height * 0.5)).cos() - half_turn.cos()) / kappa;
        let deepest = depth(height * 0.5);
        // フラップの長さ（折り線から先）。深さの 1.8 倍にして、向かいの面の近くまで届かせる。
        const REACH: f32 = 1.8;
        // 背面のフラップは、正面のフラップの下に入る。そのぶん折り線を少し内側へずらす。
        let under = 1.5 * t;
        let v_at = |k: usize| height * k as f32 / SLICES as f32;

        // 面の輪郭。左右の端が、折り線の形にへこんでいる。sign = 1 が正面（上側）、-1 が背面（下側）。
        let face = |sign: f32, shift: f32| -> Vec<P2> {
            let mut outline: Vec<P2> = (0..=SLICES).map(|k| [shift + depth(v_at(k)), sign * v_at(k)]).collect();
            outline.extend((0..=SLICES).rev().map(|k| [len - shift - depth(v_at(k)), sign * v_at(k)]));
            ccw(outline)
        };
        // フラップの輪郭。折り線と、その先の縁に挟まれたレンズ形。side = -1 が左端、1 が右端。
        let flap = |sign: f32, shift: f32, side: f32| -> (Vec<P2>, [P2; 3], Vec<P2>) {
            let edge = if side < 0.0 { shift } else { len - shift };
            let crease = |v: f32| edge - side * depth(v);
            let line: Vec<P2> = (0..=SLICES).map(|k| [crease(v_at(k)), sign * v_at(k)]).collect();
            let mut outline = line.clone();
            outline.extend((0..=SLICES).rev().map(|k| [crease(v_at(k)) + side * REACH * depth(v_at(k)), sign * v_at(k)]));
            (ccw(outline), [[crease(0.0), 0.0], [crease(height * 0.5), sign * height * 0.5], [crease(height), sign * height]], line)
        };

        let mut panels = vec![Panel {
            name: "正面",
            outline: face(1.0, 0.0),
            hinge: None,
            bend: Some(Bend { curvature: kappa, window: MAIN, line: Some(([0.0, 0.0], [len, 0.0])) }),
            curved: None,
            slices: SLICES,
            hidden_when_closed: false,
        }];
        panels.push(Panel {
            name: "背面",
            outline: face(-1.0, under),
            hinge: Some(Hinge { parent: 0, a: [0.0, 0.0], b: [len, 0.0], angle: std::f32::consts::PI - psi, window: MAIN, motion: Motion::Fixed }),
            bend: Some(Bend { curvature: kappa, window: MAIN, line: None }),
            curved: None,
            slices: SLICES,
            hidden_when_closed: false,
        });

        // 持ち手。上で合わさった 2 枚を、山折りでまっすぐ上に立てて貼り合わせる。
        let handle_height = (0.35 * height).clamp(25.0, 50.0);
        let (h0, h1) = (len * 0.27, len * 0.73);
        let inset = (h1 - h0) * 0.09;
        for (name, parent, sign) in [("正面の持ち手", 0usize, 1.0f32), ("背面の持ち手", 1usize, -1.0f32)] {
            let edge = sign * height;
            panels.push(Panel::flat(
                name,
                ccw(vec![[h0, edge], [h1, edge], [h1 - inset, edge + sign * handle_height], [h0 + inset, edge + sign * handle_height]]),
                Some(Hinge { parent, a: [h0, edge], b: [h1, edge], angle: -half_turn, window: MAIN, motion: Motion::Fixed }),
                false,
            ));
        }

        // 左右のフラップ。正面のものが外側に来る。
        for (name, parent, sign, shift, hidden) in [("正面のフラップ", 0usize, 1.0f32, 0.0f32, false), ("背面のフラップ", 1usize, -1.0f32, under, true)] {
            for side in [-1.0f32, 1.0] {
                let (outline, crease, line) = flap(sign, shift, side);
                panels.push(Panel { name, outline, hinge: None, bend: None, curved: Some(CurvedFold { parent, crease, line }), slices: SLICES, hidden_when_closed: hidden });
            }
        }

        let (min, max) = bounds(&panels);
        let r = len.min(height) * 0.26;
        let marks = vec![([len * 0.5, height * 0.5], r), ([deepest, height * 0.5], r * 0.55)];
        let hole = |sign: f32| [len * 0.5, sign * (height + handle_height * 0.5), (h1 - h0) * 0.3, handle_height * 0.22];
        // 根の面は下の折り線から巻きはじめるので、そのままだと全体が後ろへ傾く。ふくらみの半分だけ手前へ起こして、まっすぐ立てる。
        Template {
            panels,
            thickness: t,
            min,
            max,
            world: upright_world([len * 0.5, height * 0.5], deepest * 2.0, t),
            tilt: half_turn,
            tilt_window: MAIN,
            marks,
            holes: vec![hole(1.0), hole(-1.0)],
            open_angle: 0.0,
        }
    }

    /// 折り線の軸。正の角度で子パネルが内側（+n）へ動く向きを選ぶ。
    /// 軸の位置は、谷折りなら内側の面、山折りなら外側の面。
    fn hinge_axis(&self, panel: &Panel, hinge: &Hinge) -> (V3, V3) {
        self.axis_towards(panel, hinge.a, hinge.b, if hinge.is_mountain() { 0.0 } else { self.thickness })
    }

    /// 線 a-b を軸に、パネルの中身が正の角度で内側（+n）へ動く向きを選ぶ。
    fn axis_towards(&self, panel: &Panel, a: P2, b: P2, n: f32) -> (V3, V3) {
        let origin = [a[0], a[1], n];
        let mut dir = normalize([b[0] - a[0], b[1] - a[1], 0.0]);
        let c = centroid(&panel.outline);
        let velocity = cross(dir, sub([c[0], c[1], origin[2]], origin));
        if velocity[2] < 0.0 {
            dir = [-dir[0], -dir[1], -dir[2]];
        }
        (origin, dir)
    }

    fn bend_now(&self, panel: &Panel, bend: &Bend, hinge_line: Option<(P2, P2)>, fold: f32) -> Option<BendNow> {
        let (a, b) = bend.line.or(hinge_line)?;
        let (origin, dir) = self.axis_towards(panel, a, b, self.thickness);
        Some(BendNow { origin, s_dir: cross([0.0, 0.0, 1.0], dir), axis: dir, kappa: bend.curvature * progress(bend.window, fold) })
    }

    /// 折り線の今の角度。組み立てで折るぶんと、開け閉めで動くぶんの合計。
    /// 開け閉めの動きは、組み立てが終わってから効く。
    fn hinge_angle(&self, hinge: &Hinge, fold: f32, closing: f32) -> f32 {
        let theta = self.open_angle * closing.clamp(0.0, 1.0) * progress((0.95, 1.0), fold);
        hinge.angle * progress(hinge.window, fold) + hinge.motion.angle(theta)
    }

    /// 各パネルの置かれ方。`fold` は組み立ての進み具合（0 が展開図、1 が組み上がった箱）。
    /// `closing` は開け閉め（0 が開いた状態、1 が閉じた状態）で、開け閉めの動きを持つ型だけに効く。
    pub fn transforms(&self, fold: f32, closing: f32) -> Vec<Placement> {
        let mut out: Vec<Placement> = Vec::with_capacity(self.panels.len());
        for panel in &self.panels {
            if let Some(curved) = &panel.curved {
                // 親の置かれ方をそのまま受け継ぎ、折り線の 3 点が乗る平面で映す。
                // 展開図のとき（親が平ら）は、その平面が紙の面と一致するので、何も起きない。
                let parent = &out[curved.parent];
                let t = self.thickness;
                let q = curved.crease.map(|c| parent.point([c[0], c[1], t]));
                let normal = normalize(cross(sub(q[1], q[0]), sub(q[2], q[0])));
                let offset = 2.0 * dot(normal, q[0]);
                let mut reflect = Affine::IDENTITY;
                for r in 0..3 {
                    for c in 0..3 {
                        reflect.m[r][c] -= 2.0 * normal[r] * normal[c];
                    }
                    reflect.t[r] = offset * normal[r];
                }
                let flip_thickness = Affine { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]], t: [0.0, 0.0, 2.0 * t] };
                let mirror = if dot(normal, normal) > 0.5 { Some((reflect, flip_thickness)) } else { None };
                out.push(Placement { base: parent.base, bend: parent.bend, mirror });
                continue;
            }
            let placement = match &panel.hinge {
                None => Placement { base: Affine::IDENTITY, bend: panel.bend.as_ref().and_then(|b| self.bend_now(panel, b, None, fold)), mirror: None },
                Some(hinge) => {
                    let (origin, dir) = self.hinge_axis(panel, hinge);
                    // 親が丸まっているときは、折り線の位置での向きを土台にする（折り線は親の巻く軸と平行である前提）。
                    let parent = out[hinge.parent].at(lerp(hinge.a, hinge.b, 0.5));
                    let base = parent.then_after(&Affine::rotation_about(origin, dir, self.hinge_angle(hinge, fold, closing)));
                    let bend = panel.bend.as_ref().and_then(|b| self.bend_now(panel, b, Some((hinge.a, hinge.b)), fold));
                    Placement { base, bend, mirror: None }
                }
            };
            out.push(placement);
        }
        out
    }

    /// 紙の座標から表示用の座標への変換。`fold` が進むにつれて、型によっては全体を倒す。
    pub fn world_at(&self, fold: f32) -> Affine {
        if self.tilt == 0.0 {
            return self.world;
        }
        Affine::rotation_about([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], self.tilt * progress(self.tilt_window, fold)).then_after(&self.world)
    }

    fn uv(&self, p: P2) -> [f32; 2] {
        [(p[0] - self.min[0]) / (self.max[0] - self.min[0]), (self.max[1] - p[1]) / (self.max[1] - self.min[1])]
    }

    /// 三角形の並び（3 頂点ずつ）で、厚みのある紙の形を作る。
    pub fn build_mesh(&self, fold: f32, closing: f32) -> Vec<Vertex> {
        let t = self.thickness;
        let placements = self.transforms(fold, closing);
        let world = self.world_at(fold);
        let mut out = Vec::new();
        let mut emit = |frame: &Affine, p: V3, normal: V3, uv: [f32; 2], kind: f32| {
            out.push(Vertex { pos: world.point(frame.point(p)), normal: world.vector(frame.vector(normal)), uv, kind });
        };

        for (panel, place) in self.panels.iter().zip(&placements) {
            let pts = &panel.outline;
            // 丸めたパネルは、折り線に平行な細片に分けてから面を張る。
            let kappa = place.active_bend().map(|b| b.kappa.abs());
            let pieces: Vec<Vec<P2>> = match (place.distance(pts[0]), kappa, panel.slices) {
                (None, _, _) | (Some(_), None, 0) => vec![pts.clone()],
                (Some(_), kappa, slices) => {
                    let dist: Vec<f32> = pts.iter().map(|&p| place.distance(p).unwrap_or(0.0)).collect();
                    let (lo, hi) = (dist.iter().copied().fold(f32::INFINITY, f32::min), dist.iter().copied().fold(f32::NEG_INFINITY, f32::max));
                    let count = if slices > 0 { slices } else { ((kappa.unwrap_or(0.0) * (hi - lo) / STRIP_ANGLE).ceil() as usize).clamp(1, 64) };
                    let step = (hi - lo) / count as f32;
                    (0..count)
                        .map(|k| {
                            let (from, to) = (lo + step * k as f32, lo + step * (k + 1) as f32);
                            let lower = clip(pts, |p| place.distance(p).unwrap_or(0.0) - from);
                            clip(&lower, |p| to - place.distance(p).unwrap_or(0.0))
                        })
                        .filter(|piece| piece.len() >= 3)
                        .collect()
                }
            };
            for piece in &pieces {
                for i in 1..piece.len() - 1 {
                    for &q in &[piece[0], piece[i], piece[i + 1]] {
                        emit(&place.at(q), [q[0], q[1], 0.0], [0.0, 0.0, -1.0], self.uv(q), 0.0);
                    }
                    for &q in &[piece[0], piece[i], piece[i + 1]] {
                        emit(&place.at(q), [q[0], q[1], t], [0.0, 0.0, 1.0], self.uv(q), 1.0);
                    }
                }
            }
            // 断面。丸めたパネルでは、辺も細かく分ける。
            for i in 0..pts.len() {
                let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                let normal = normalize([q[1] - p[1], p[0] - q[0], 0.0]);
                let count = match (place.active_bend(), place.distance(p), place.distance(q)) {
                    (Some(b), Some(sp), Some(sq)) => ((b.kappa.abs() * (sq - sp).abs() / STRIP_ANGLE).ceil() as usize).clamp(1, 64),
                    _ => 1,
                };
                for k in 0..count {
                    let (a, b) = (lerp(p, q, k as f32 / count as f32), lerp(p, q, (k + 1) as f32 / count as f32));
                    let quad = [[a[0], a[1], 0.0], [b[0], b[1], 0.0], [b[0], b[1], t], [a[0], a[1], t]];
                    for idx in [0usize, 1, 2, 0, 2, 3] {
                        emit(&place.at([quad[idx][0], quad[idx][1]]), quad[idx], normal, self.uv(a), 2.0);
                    }
                }
            }
            // 折り線のすき間を円弧で埋める。谷折りは外側に、山折りは内側にできる。
            if let Some(hinge) = &panel.hinge {
                let angle = self.hinge_angle(hinge, fold, closing);
                if angle.abs() > 1e-4 {
                    let parent = placements[hinge.parent].at(lerp(hinge.a, hinge.b, 0.5));
                    let (origin, dir) = self.hinge_axis(panel, hinge);
                    let (start, kind) = if angle > 0.0 { ([0.0, 0.0, -t], 0.0) } else { ([0.0, 0.0, t], 1.0) };
                    let steps = ((angle.abs() / std::f32::consts::FRAC_PI_2) * 6.0).ceil().max(2.0) as usize;
                    let (a3, b3) = ([hinge.a[0], hinge.a[1], origin[2]], [hinge.b[0], hinge.b[1], origin[2]]);
                    let along = normalize(sub(b3, a3));
                    let ring: Vec<V3> = (0..=steps).map(|j| rotate(start, dir, angle * j as f32 / steps as f32)).collect();
                    for j in 0..steps {
                        let (o0, o1) = (ring[j], ring[j + 1]);
                        let (n0, n1) = (normalize(o0), normalize(o1));
                        let quad = [(add(a3, o0), n0, hinge.a), (add(b3, o0), n0, hinge.b), (add(b3, o1), n1, hinge.b), (add(a3, o1), n1, hinge.a)];
                        for idx in [0usize, 1, 2, 0, 2, 3] {
                            emit(&parent, quad[idx].0, quad[idx].1, self.uv(quad[idx].2), kind);
                        }
                        for (center, sign, uv_at) in [(a3, -1.0f32, hinge.a), (b3, 1.0f32, hinge.b)] {
                            for p in [center, add(center, o0), add(center, o1)] {
                                emit(&parent, p, scale(along, sign), self.uv(uv_at), 2.0);
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
        for tpl in [Template::tuck_box(W, D, H, T), Template::prism(5, 70.0, 120.0, 2.0), Template::carry_bag(150.0, 60.0, 160.0, 0.5, 0.7), Template::pillow(140.0, 90.0, 0.4, 1.0)] {
            for m in tpl.transforms(0.0, 1.0) {
                close(m.point([12.0, 34.0, 1.0]), [12.0, 34.0, 1.0]);
            }
        }
    }

    #[test]
    fn outlines_are_counter_clockwise() {
        for tpl in [Template::tuck_box(W, D, H, T), Template::carry_bag(150.0, 60.0, 160.0, 0.5, 0.7)] {
            for p in &tpl.panels {
                assert!(signed_area(&p.outline) > 0.0, "{} が時計回り", p.name);
            }
        }
    }

    #[test]
    fn walls_close_into_a_box() {
        let tpl = Template::tuck_box(W, D, H, T);
        let m = tpl.transforms(1.0, 1.0);
        let (u_l, u_f, u_r, u_b) = layout();

        close(m[index(&tpl, "右側面")].point([u_r + D, 50.0, T]), [u_r, 50.0, T + D]);
        close(m[index(&tpl, "左側面")].point([u_f - 20.0, 50.0, T]), [u_f, 50.0, T + 20.0]);
        close(m[index(&tpl, "背面")].point([u_b + W + T, 50.0, T]), [u_f - T, 50.0, T + D]);
        close(m[index(&tpl, "のりしろ")].point([u_l - 5.0, 50.0, 0.0]), [u_f + 5.0, 50.0, T + D]);
    }

    #[test]
    fn lid_sits_on_flaps_and_tuck_goes_inside_front() {
        let tpl = Template::tuck_box(W, D, H, T);
        let m = tpl.transforms(1.0, 1.0);
        let (u_l, _u_f, u_r, u_b) = layout();

        let flap = m[index(&tpl, "上の左フラップ")].point([u_l + 10.0, H + 8.0, T]);
        assert!((flap[1] - H).abs() < 1e-2, "フラップの高さ {}", flap[1]);
        close(m[index(&tpl, "上のフタ")].point([u_b + 30.0, H + T + 10.0, T]), [u_r - 30.0, H + T, T + D - 10.0]);
        close(m[index(&tpl, "上の差し込み")].point([u_b + 30.0, H + D + 6.0, 0.0]), [u_r - 30.0, H + T - 6.0, T]);
        close(m[index(&tpl, "底の差し込み")].point([u_b + 30.0, -D - 6.0, 0.0]), [u_r - 30.0, -T + 6.0, T]);
    }

    #[test]
    fn prisms_close_for_several_side_counts() {
        let (side, h, t) = (70.0f32, 120.0f32, 2.0f32);
        for n in [3usize, 5, 6, 8] {
            let tpl = Template::prism(n, side, h, t);
            for p in &tpl.panels {
                assert!(signed_area(&p.outline) > 0.0, "{n} 角柱の {} が時計回り", p.name);
            }
            let m = tpl.transforms(1.0, 1.0);
            let exterior = std::f32::consts::TAU / n as f32;
            let shift = t / exterior.sin();
            let u0 = shift;

            let last_end = u0 + n as f32 * side - shift;
            let on_wall = m[index(&tpl, "のりしろ")].point([last_end + 3.0, h * 0.5, 0.0]);
            assert!((on_wall[2] - t).abs() < 1e-2, "{n} 角柱: のりしろが側面から {} 離れている", on_wall[2] - t);
            let last_tip = m[n - 1].point([last_end, h * 0.5, t]);
            assert!((last_tip[2] - 2.0 * t).abs() < 1e-2, "{n} 角柱: 最後の側面の端 {last_tip:?}");

            let corner = [u0 + side + side * exterior.cos(), h + t + side * exterior.sin()];
            let lid_point = m[index(&tpl, "上のフタ")].point([corner[0], corner[1], t]);
            let wall_point = m[1].point([u0 + 2.0 * side, h, t]);
            close(lid_point, [wall_point[0], wall_point[1] + t, wall_point[2]]);
        }
    }

    #[test]
    fn carry_bag_meets_at_the_top() {
        let (w, d, l, t) = (150.0f32, 60.0f32, 160.0f32, 0.5f32);
        for bulge in [0.0f32, 0.4, 0.8, 1.2] {
            let tpl = Template::carry_bag(w, d, l, t, bulge);
            let m = tpl.transforms(1.0, 1.0);
            let c = w * 0.5;

            // 2 枚の持ち手の内側どうしが、中央（z = 0）でぴったり合わさる。高さもそろう。
            let front = tpl.world_at(1.0).point(m[index(&tpl, "正面の持ち手")].point([c, d + l + 10.0, t]));
            let back = tpl.world_at(1.0).point(m[index(&tpl, "背面の持ち手")].point([c, -l - 10.0, t]));
            assert!(front[2].abs() < 2e-2 && back[2].abs() < 2e-2, "ふくらみ {bulge}: 持ち手が中央からずれている {front:?} {back:?}");
            assert!((front[1] - back[1]).abs() < 2e-2 && front[0].abs() < 1e-2);
            // 持ち手はまっすぐ上に立つ（10 mm 先の点が、真上に 10 mm）。
            let above = tpl.world_at(1.0).point(m[index(&tpl, "正面の持ち手")].point([c, d + l + 20.0, t]));
            close(above, [front[0], front[1] + 10.0, front[2]]);

            // 正面を丸めても、紙の上での長さは変わらない（伸び縮みしない）。
            let front_panel = &m[index(&tpl, "正面")];
            let mut length = 0.0;
            let mut prev = front_panel.point([c, d, t]);
            for i in 1..=400 {
                let next = front_panel.point([c, d + l * i as f32 / 400.0, t]);
                length += crate::math::length(sub(next, prev));
                prev = next;
            }
            assert!((length - l).abs() < 0.05, "ふくらみ {bulge}: 長さ {length} が {l} と違う");

            // 側面は、底の端からまっすぐ立つ。
            close(tpl.world_at(1.0).point(m[index(&tpl, "右の側面")].point([w + 20.0, d * 0.5, t])), [w * 0.5, 20.0, 0.0]);
        }
    }

    #[test]
    fn gable_roof_closes_and_the_corner_linkage_stays_connected() {
        let (w, d, h, roof, t) = (160.0f32, 100.0f32, 110.0f32, 45.0f32, 0.5f32);
        let tpl = Template::gable(w, d, h, roof, t);
        for p in &tpl.panels {
            assert!(signed_area(&p.outline) > 0.0, "{} が時計回り", p.name);
        }
        let glue = (0.8 * (d - t)).clamp(4.0, 15.0);
        let u_f = glue + (d - t);
        let (u_r, u_b) = (u_f + w, u_f + w + d);
        let slope = ((d * 0.5).powi(2) + roof * roof).sqrt();
        let ridge = h + slope;
        let front_roof = index(&tpl, "正面の屋根");
        let back_roof = index(&tpl, "背面の屋根");
        let center = index(&tpl, "右の三角");
        let wing = index(&tpl, "右の折り込み");
        let apex = [(u_r + u_b) * 0.5, ridge, t];

        // 閉じた状態: 正面と背面の屋根の先が、奥行きの中央、屋根の高さで合わさる。
        let m = tpl.transforms(1.0, 1.0);
        close(m[front_roof].point([u_f + 30.0, ridge, t]), [u_f + 30.0, h + roof, t + d * 0.5]);
        close(m[back_roof].point([u_b + 30.0, ridge, t]), [u_r - 30.0, h + roof, t + d * 0.5]);
        // 側面の三角形の頂点は、奥行きの半分だけ内側へ倒れて、屋根のてっぺんの線に乗る。
        close(m[center].point(apex), [u_r - d * 0.5, h + roof, t + d * 0.5]);

        // 開いた状態: 屋根は胴の面のまま、まっすぐ上に立つ。
        let m = tpl.transforms(1.0, 0.0);
        close(m[front_roof].point([u_f + 30.0, ridge, t]), [u_f + 30.0, ridge, t]);
        close(m[center].point(apex), [u_r, ridge, t + d * 0.5]);

        // 開け閉めの途中でも、側面の三角形と折り込む三角形は、共有する頂点でつながったまま。
        // 3 枚が連動する角度の式が正しいことの確認。
        for closing in [0.0f32, 0.2, 0.5, 0.8, 1.0] {
            let m = tpl.transforms(1.0, closing);
            let a = m[center].point(apex);
            let b = m[wing].point(apex);
            for i in 0..3 {
                assert!((a[i] - b[i]).abs() < 0.05, "閉じ具合 {closing}: 頂点が離れている {a:?} {b:?}");
            }
        }

        // 組み立ての途中（まだ筒になっていない）では、開け閉めの動きは効かない。
        let half_built = tpl.transforms(0.5, 1.0);
        let reference = tpl.transforms(0.5, 0.0);
        close(half_built[front_roof].point([u_f + 30.0, ridge, t]), reference[front_roof].point([u_f + 30.0, ridge, t]));
    }

    #[test]
    fn pillow_closes_and_flaps_fold_across_a_plane() {
        let (len, height, t) = (140.0f32, 90.0f32, 0.4f32);
        for bulge in [0.6f32, 1.0, 1.4] {
            let tpl = Template::pillow(len, height, t, bulge);
            for p in &tpl.panels {
                assert!(signed_area(&p.outline) > 0.0, "{} が時計回り", p.name);
            }
            let kappa = bulge / height;
            let half_turn = bulge * 0.5;
            let depth = |v: f32| ((kappa * (v - height * 0.5)).cos() - half_turn.cos()) / kappa;
            let m = tpl.transforms(1.0, 1.0);
            let (front, back) = (index(&tpl, "正面"), index(&tpl, "背面"));

            // 2 枚の面の上の端が、内側の面どうしでぴったり合わさる。
            close(m[front].point([len * 0.5, height, t]), m[back].point([len * 0.5, -height, t]));
            // 丸めても、紙の上での高さは変わらない。
            let mut length = 0.0;
            let mut prev = m[front].point([len * 0.5, 0.0, t]);
            for i in 1..=300 {
                let next = m[front].point([len * 0.5, height * i as f32 / 300.0, t]);
                length += crate::math::length(sub(next, prev));
                prev = next;
            }
            assert!((length - height).abs() < 0.05, "ふくらみ {bulge}: 高さ {length} が {height} と違う");

            // 2 枚の持ち手の内側どうしが、紙の厚みの範囲で合わさる。
            let front_handle = m[index(&tpl, "正面の持ち手")].point([len * 0.5, height + 10.0, t]);
            let back_handle = m[index(&tpl, "背面の持ち手")].point([len * 0.5, -height - 10.0, t]);
            assert!(crate::math::length(sub(front_handle, back_handle)) < 2.0 * t, "ふくらみ {bulge}: 持ち手が離れている");

            // 正面の左のフラップ。折り線の上では、面とぴったりつながる（折り線が 1 枚の平面に乗っている）。
            let flap = tpl.panels.iter().position(|p| p.name == "正面のフラップ").unwrap();
            // 奥行き方向（2 枚が合わさる平面に垂直で、背面へ向かう向き）。丸める前の紙の座標で表す。
            let inward = [0.0, -half_turn.sin(), half_turn.cos()];
            for v in [height * 0.2, height * 0.5, height * 0.8] {
                let on_crease = m[front].point([depth(v), v, t]);
                close(m[flap].point([depth(v), v, t]), on_crease);
                // フラップは、折り線から奥行き方向へまっすぐ倒れ込む。折り線から 5 mm 先の点は、奥へ 5 mm。
                let beyond = m[flap].point([depth(v) - 5.0, v, t]);
                close(beyond, [on_crease[0] + 5.0 * inward[0], on_crease[1] + 5.0 * inward[1], on_crease[2] + 5.0 * inward[2]]);
            }
        }
    }

    #[test]
    fn pillow_flaps_stay_attached_while_assembling() {
        // 組み立ての途中では、折り線は 1 枚の平面にぴったりとは乗らない。それでも、フラップと面のずれは目に見えない大きさに収まる。
        let (len, height, t) = (180.0f32, 110.0f32, 0.5f32);
        for bulge in [0.5f32, 1.2, 1.57] {
            let tpl = Template::pillow(len, height, t, bulge);
            let kappa = bulge / height;
            let depth = |v: f32| ((kappa * (v - height * 0.5)).cos() - (bulge * 0.5).cos()) / kappa;
            let front = index(&tpl, "正面");
            let flap = tpl.panels.iter().position(|p| p.name == "正面のフラップ").unwrap();
            for fold in [0.1f32, 0.25, 0.5, 0.75, 0.9] {
                let m = tpl.transforms(fold, 1.0);
                for k in 0..=40 {
                    let v = height * k as f32 / 40.0;
                    let gap = crate::math::length(sub(m[flap].point([depth(v), v, t]), m[front].point([depth(v), v, t])));
                    assert!(gap < 0.3, "ふくらみ {bulge}、組み立て {fold}: 折り線で {gap} mm 離れている");
                }
            }
        }
    }

    #[test]
    fn mesh_is_a_triangle_list() {
        for tpl in [Template::tuck_box(W, D, H, T), Template::prism(3, 70.0, 120.0, 2.0), Template::carry_bag(150.0, 60.0, 160.0, 0.5, 0.7), Template::gable(160.0, 100.0, 110.0, 45.0, 0.5), Template::pillow(140.0, 90.0, 0.4, 1.0)] {
            for fold in [0.0, 0.3, 0.6, 1.0] {
                let mesh = tpl.build_mesh(fold, 0.6);
                assert!(!mesh.is_empty() && mesh.len() % 3 == 0);
                assert!(mesh.iter().all(|v| v.pos.iter().all(|c| c.is_finite())));
            }
        }
    }
}
