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

mod library;

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
    /// ゲーブル型の、側面の三角形と折り込む三角形の間の、斜めの折り線。山折り。値は `GableCenter` と同じ。
    GableDiagonal(f32),
}

/// ゲーブル型の角で、側面の三角形と折り込む三角形が、斜めの折り線でなす角度（0 なら同じ平面）。
/// 屋根の傾き `theta` と、側面の三角形の倒れ `gable_center_angle` から、2 枚の面の向きを求めて測る。
fn gable_diagonal_angle(theta: f32, k: f32) -> f32 {
    let psi = gable_center_angle(theta, k);
    // 角を原点、正面の上の辺を x、上を y、奥を -z に取る。側面は x = 0 の面で、外向きが +x。
    let roof_up = [0.0, theta.cos(), -theta.sin()];
    let center_normal = [psi.cos(), psi.sin(), 0.0];
    // 斜めの折り線の向き（側面の三角形の、底辺の中央の上にある頂点へ）。
    let diagonal = normalize([-psi.sin(), psi.cos(), -k]);
    // 折り込む三角形は、屋根の横の辺と、斜めの折り線を含む面。
    let wing_normal = normalize(cross(diagonal, roof_up));
    dot(center_normal, wing_normal).clamp(-1.0, 1.0).acos()
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
            Motion::GableDiagonal(k) => -gable_diagonal_angle(theta, k),
        }
    }
}

impl Hinge {
    fn is_mountain(&self) -> bool {
        self.angle < 0.0 || matches!(self.motion, Motion::Follow(scale) if scale < 0.0) || matches!(self.motion, Motion::GableDiagonal(_))
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
    /// 箱の中での役割。
    pub role: Role,
}

/// パネルの役割。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// 箱の面。組み上がっても外から見える。
    Face,
    /// のりしろ。隣の面の内側に貼って、固定する。
    GlueTab,
    /// 差し込み（ベロ）。フタの先に付いて、面の内側に入り、フタを留める。
    Tuck,
    /// フタの下に入る小さいフラップ。すき間をふさぐ。
    Flap,
    /// 内側へ折り込まれる面（ゲーブル型の屋根の裏へ入る三角形など）。組み上がると隠れる。
    Gusset,
    /// 貼りしろ。2 枚を背中合わせに貼って口を閉じる縁（ピロー型の上の端）。外から見える。
    Fin,
}

/// 開け口が、閉じた状態を何で保っているか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// 差し込み（そのパネルの番号）。
    Tuck(usize),
    /// 紙の張り（ピロー型の端のフラップ）。
    Tension,
    /// 留め具やかみ合わせ（ゲーブル型の持ち手の留め）。形は型紙にまだ入っていない。
    Latch,
    /// 何も留めていない。型紙の不備。
    Nothing,
}

/// 開け口。お客さんが開けるパネルと、それを留めているもの。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening {
    pub lid: usize,
    pub hold: Hold,
}

impl Panel {
    fn flat(name: &'static str, outline: Vec<P2>, hinge: Option<Hinge>, role: Role) -> Panel {
        Panel { name, outline, hinge, bend: None, curved: None, slices: 0, role }
    }

    /// 組み上がると隠れる部分（のりしろ、差し込みなど）か。
    pub fn hidden_when_closed(&self) -> bool {
        !matches!(self.role, Role::Face | Role::Fin)
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
    /// 開け口。空なら、開け口が決まっていない（型紙の不備）。
    pub openings: Vec<Opening>,
    /// 貼り合わせる面の組（パネルの番号）。のりしろと貼る相手、貼りしろどうし、など。
    pub glued: Vec<(usize, usize)>,
}

/// 多角形の内側か。点から右へ伸ばした線が辺を横切る回数で判定する（へこんだ輪郭でも使える）。
pub fn inside(outline: &[P2], p: P2) -> bool {
    let mut hit = false;
    for i in 0..outline.len() {
        let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            hit = !hit;
        }
    }
    hit
}

/// 名前でパネルを探して、貼り合わせる面の組を作る。
fn glued(panels: &[Panel], list: &[(&str, &str)]) -> Vec<(usize, usize)> {
    let find = |name: &str| panels.iter().position(|p| p.name == name);
    list.iter().filter_map(|&(a, b)| Some((find(a)?, find(b)?))).collect()
}

/// 多角形の辺を、辺ごとに決めた量だけ内側へ寄せる。辺 i は、頂点 i から次の頂点まで。
/// `inward` は、頂点の並びが反時計回りなら 1、時計回りなら -1。
fn inset_edges(points: &[P2], amount: &[f32], inward: f32) -> Vec<P2> {
    let n = points.len();
    // 寄せたあとの辺が乗る線（通る点と向き）。
    let lines: Vec<(P2, P2)> = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt().max(1e-9);
            let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let shift = amount[i] * inward;
            ([a[0] - dir[1] * shift, a[1] + dir[0] * shift], dir)
        })
        .collect();
    // 頂点 i は、前の辺と辺 i の交点。
    (0..n)
        .map(|i| {
            let ((p, d), (q, e)) = (lines[(i + n - 1) % n], lines[i]);
            let cross = d[0] * e[1] - d[1] * e[0];
            if cross.abs() < 1e-6 {
                return q;
            }
            let s = ((q[0] - p[0]) * e[1] - (q[1] - p[1]) * e[0]) / cross;
            [p[0] + d[0] * s, p[1] + d[1] * s]
        })
        .collect()
}

/// 名前でパネルを探して、開け口の一覧を作る。
fn openings(panels: &[Panel], list: &[(&str, Option<&str>, Hold)]) -> Vec<Opening> {
    let find = |name: &str| panels.iter().position(|p| p.name == name);
    list.iter()
        .filter_map(|&(lid, tuck, hold)| {
            let hold = match tuck.and_then(find) {
                Some(index) => Hold::Tuck(index),
                None => hold,
            };
            find(lid).map(|lid| Opening { lid, hold })
        })
        .collect()
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

    /// 抜き穴の中か。角の丸い四角として判定する。
    pub fn in_hole(&self, u: f32, v: f32) -> bool {
        self.holes.iter().any(|h| ((u - h[0]) / h[2]).powi(4) + ((v - h[1]) / h[3]).powi(4) < 1.0)
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
    fn prism_tucks_go_inside_the_walls() {
        let (side, h) = (70.0f32, 120.0f32);
        for n in [3usize, 4, 5, 6, 8] {
            for t in [0.5f32, 2.0] {
                let tpl = Template::prism(n, side, h, t);
                let m = tpl.transforms(1.0, 1.0);
                let lid = index(&tpl, "上のフタ");
                let tucks: Vec<usize> = (0..tpl.panels.len()).filter(|&i| tpl.panels[i].name == "上の差し込み").collect();
                assert_eq!(tucks.len(), if n % 2 == 0 { 1 } else { 2 }, "{n} 角柱");
                for &tuck in &tucks {
                    let hinge = tpl.panels[tuck].hinge.as_ref().unwrap();
                    assert_eq!(hinge.parent, lid);
                    // 差し込みの先（つけ根から遠い 2 点の中央）。
                    let mid = [(hinge.a[0] + hinge.b[0]) * 0.5, (hinge.a[1] + hinge.b[1]) * 0.5];
                    let dir = [hinge.b[0] - hinge.a[0], hinge.b[1] - hinge.a[1]];
                    let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
                    let off_hinge = |p: &P2| ((p[0] - hinge.a[0]) * dir[1] - (p[1] - hinge.a[1]) * dir[0]) / len;
                    let ends: Vec<P2> = tpl.panels[tuck].outline.iter().copied().filter(|p| off_hinge(p).abs() > 1.0).collect();
                    assert_eq!(ends.len(), 2);
                    let far = [(ends[0][0] + ends[1][0]) * 0.5, (ends[0][1] + ends[1][1]) * 0.5];
                    let depth = off_hinge(&far).abs();
                    let tip = [mid[0] + (far[0] - mid[0]) * 0.8, mid[1] + (far[1] - mid[1]) * 0.8];
                    // 差し込みが入る側面を探す: 差し込みの外側の面が、その側面の内側の面にぴったり付く。
                    let outer = m[tuck].point([tip[0], tip[1], 0.0]);
                    let touching = (1..n).filter(|&wall| {
                        let o = &tpl.panels[wall].outline;
                        let c = [(o[0][0] + o[2][0]) * 0.5, h * 0.5];
                        let (p0, p1) = (m[wall].point([c[0], c[1], t]), m[wall].point([c[0], c[1], t + 1.0]));
                        let normal = sub(p1, p0);
                        dot(sub(outer, p0), normal).abs() < 0.05
                    }).count();
                    assert!(touching >= 1, "{n} 角柱、厚み {t}: 差し込みが側面の内側に付いていない");
                    // 差し込みは、フタから下へ向かう（箱の中へ入る）。
                    let top = m[lid].point([mid[0], mid[1], t]);
                    assert!(top[1] - outer[1] > depth * 0.5, "{n} 角柱: 差し込みが下を向いていない {top:?} {outer:?}");
                }
            }
        }
    }

    #[test]
    fn gable_far_wing_stays_on_the_back_roof() {
        let (w, d, h, roof) = (160.0f32, 100.0f32, 110.0f32, 45.0f32);
        for t in [0.02f32, 0.5] {
            let tpl = Template::gable(w, d, h, roof, t);
            let glue = (0.8 * (d - t)).clamp(4.0, 15.0);
            let (u_l, u_f) = (glue, glue + (d - t));
            let u_b = u_f + w + d;
            let slope = ((d * 0.5).powi(2) + roof * roof).sqrt();
            let ridge = h + slope;
            let back_roof = index(&tpl, "背面の屋根");
            let tab = index(&tpl, "屋根ののりしろ");
            let wing = tpl.panels[tab].hinge.as_ref().unwrap().parent;
            assert_eq!(tpl.panels[wing].name, "左の折り込み");
            // 開け閉めのどの段階でも、左奥の折り込みの縦の辺は、背面の屋根の端の辺と重なる（紙の厚みの範囲で）。
            for closing in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
                let m = tpl.transforms(1.0, closing);
                for v in [h + 5.0, (h + ridge) * 0.5, ridge - 5.0] {
                    let on_wing = m[wing].point([u_l, v, t]);
                    let on_roof = m[back_roof].point([u_b + w, v, t]);
                    let gap = crate::math::length(sub(on_wing, on_roof));
                    assert!(gap < 2.5 * t + 0.3, "厚み {t}、閉じ具合 {closing}、高さ {v}: {gap} mm 離れている");
                    // のりしろは、背面の屋根の内側の面に沿っている。
                    let on_tab = m[tab].point([u_l - 4.0, v, 0.0]);
                    let (p0, p1) = (m[back_roof].point([u_b + w - 4.0, v, t]), m[back_roof].point([u_b + w - 4.0, v, t + 1.0]));
                    let off = dot(sub(on_tab, p0), sub(p1, p0));
                    assert!(off.abs() < 2.5 * t + 0.3, "厚み {t}、閉じ具合 {closing}: のりしろが屋根から {off} mm 離れている");
                }
            }
        }
    }

    #[test]
    fn pillow_fins_meet_along_the_whole_top_edge() {
        let (len, height, t) = (180.0f32, 110.0f32, 0.5f32);
        for bulge in [0.5f32, 1.0, 1.4] {
            let tpl = Template::pillow(len, height, t, bulge);
            let m = tpl.transforms(1.0, 1.0);
            let (front, back) = (index(&tpl, "正面の貼りしろ"), index(&tpl, "背面の貼りしろ"));
            // 貼りしろは、端から端まで、内側の面どうしが紙の厚みの範囲で重なる。まっすぐ上を向いている。
            for u in [5.0, len * 0.5, len - 5.0] {
                let a = m[front].point([u, height + 3.0, t]);
                let b = m[back].point([u, -height - 3.0, t]);
                assert!(crate::math::length(sub(a, b)) < 2.0 * t, "ふくらみ {bulge}: 貼りしろが離れている");
                let up = sub(m[front].point([u, height + 4.0, t]), m[front].point([u, height + 1.0, t]));
                // 巻きはじめの線から見て、2 枚が合わさる平面の向き（ふくらみの半分だけ手前へ起こした向き）。
                let expected = [0.0, 3.0 * (bulge * 0.5).cos(), 3.0 * (bulge * 0.5).sin()];
                close(up, expected);
            }
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
