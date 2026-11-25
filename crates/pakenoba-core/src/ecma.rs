//! ECMA の型（紙器の型の番号）から、型紙を作る。
//!
//! ECMA（European Carton Makers Association）の番号は、紙箱の形を呼び分けるための名前。
//! A 類（四角い筒を、のりしろで閉じる箱）の番号は「A + 底の閉じ方 . 上の閉じ方 . 底のフタの面 . 上のフタの面」。
//! 面は 01 が背面、03 が正面、00 は面によらない閉じ方。
//!
//! 形は番号と内寸から、ここで描き起こす（ECMA のカタログの図は写さない）。描いた線は、DXF の読み込みと
//! 同じ道（`Template::imported_rooted`）で折る。番号の一覧と閉じ方の分け方は、pacxio（MIT）の `engine/ecma.py` に合わせた。
//!
//! まだ簡略なもの: 1-2-3 底（55）とワンタッチ底（60）は、かみ合わせと斜めの貼りを描かず、底のフラップの外形だけ。

use pakenoba_i18n::t;

use crate::dieline::{Drawing, Group, LineKind, Path};
use crate::template::P2;
use crate::template::{ImportSettings, Template};

/// 箱の端の閉じ方（番号の 2 けた）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closure {
    /// 01: 開いたまま。
    Open,
    /// 10: 4 枚のフラップ。長い面のフラップは、箱の奥行きいっぱい（重なり合う）。
    Flaps,
    /// 11: 4 枚のフラップ。フタの面のフラップだけ奥行きいっぱいで、ほかは短い。
    Reduced,
    /// 20: 差し込みのフタと、左右のフラップ。
    Tuck,
    /// 21: 吊り下げ（番号の面を上へ伸ばし、穴をあける）と、向かいの面に付く差し込みのフタ。
    Hanger,
    /// 40: サイド差し込み。フタの左右の翼を、側面の内側へ差し込む。向かいの面に内フラップ。
    SideTuck,
    /// 55: 1-2-3 底（組み底）。
    Snap,
    /// 60: ワンタッチ底（自動で組まれる底）。
    Auto,
}

impl Closure {
    pub const ALL: [Closure; 8] = [Closure::Open, Closure::Flaps, Closure::Reduced, Closure::Tuck, Closure::Hanger, Closure::SideTuck, Closure::Snap, Closure::Auto];

    /// 番号の 2 けた。
    pub fn number(self) -> u32 {
        match self {
            Closure::Open => 1,
            Closure::Flaps => 10,
            Closure::Reduced => 11,
            Closure::Tuck => 20,
            Closure::Hanger => 21,
            Closure::SideTuck => 40,
            Closure::Snap => 55,
            Closure::Auto => 60,
        }
    }

    pub fn from_number(n: u32) -> Option<Closure> {
        Closure::ALL.into_iter().find(|c| c.number() == n)
    }

    /// 画面に出す名前。
    pub fn label(self) -> &'static str {
        match self {
            Closure::Open => t!("Open"),
            Closure::Flaps => t!("Four flaps (full overlap)"),
            Closure::Reduced => t!("Four flaps (reduced)"),
            Closure::Tuck => t!("Tuck lid"),
            Closure::Hanger => t!("Tuck lid with hanger"),
            Closure::SideTuck => t!("Side tuck lid"),
            Closure::Snap => t!("1-2-3 snap lock bottom"),
            Closure::Auto => t!("Crash lock bottom"),
        }
    }

    /// 上に使えるか（55 と 60 は底だけ）。底に使えるか（21 は上だけ）。
    pub fn on_top(self) -> bool {
        !matches!(self, Closure::Snap | Closure::Auto)
    }
    pub fn on_bottom(self) -> bool {
        !matches!(self, Closure::Hanger)
    }

    /// フタや、いちばん外のフラップがつく面を選ぶ閉じ方か（01 と 60 は選ばない）。
    pub fn has_side(self) -> bool {
        !matches!(self, Closure::Open | Closure::Auto)
    }
}

/// ECMA の A 類の番号。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Code {
    pub bottom: Closure,
    pub top: Closure,
    /// 底と上のフタがつく面。false が背面（01）、true が正面（03）。面によらない閉じ方では使わない。
    pub bottom_front: bool,
    pub top_front: bool,
}

impl Code {
    /// 数字 8 けた（A20.20.03.01 なら 20200301）から。
    pub fn from_u32(n: u32) -> Option<Code> {
        let (b, tp, bp, tpos) = (n / 1_000_000, (n / 10_000) % 100, (n / 100) % 100, n % 100);
        let bottom = Closure::from_number(b).filter(|c| c.on_bottom())?;
        let top = Closure::from_number(tp).filter(|c| c.on_top())?;
        let side = |closure: Closure, pos: u32| -> Option<bool> {
            match (closure.has_side(), pos) {
                (false, 0) => Some(false),
                (true, 1) => Some(false),
                (true, 3) => Some(true),
                _ => None,
            }
        };
        Some(Code { bottom, top, bottom_front: side(bottom, bp)?, top_front: side(top, tpos)? })
    }

    pub fn to_u32(self) -> u32 {
        let pos = |c: Closure, front: bool| if !c.has_side() { 0 } else if front { 3 } else { 1 };
        self.bottom.number() * 1_000_000 + self.top.number() * 10_000 + pos(self.bottom, self.bottom_front) * 100 + pos(self.top, self.top_front)
    }

    /// 番号の文字（A20.20.03.01）。
    pub fn name(self) -> String {
        let n = self.to_u32();
        format!("A{:02}.{:02}.{:02}.{:02}", n / 1_000_000, (n / 10_000) % 100, (n / 100) % 100, n % 100)
    }

    /// 文字の番号から（「A20.20.03.01」「a20200301」など）。
    pub fn parse(text: &str) -> Option<Code> {
        let digits: String = text.trim().trim_start_matches(['A', 'a']).chars().filter(|c| c.is_ascii_digit()).collect();
        (digits.len() == 8).then(|| digits.parse().ok()).flatten().and_then(Code::from_u32)
    }

    /// 公開の刀型の一覧に載っている番号か（載っていない組み合わせも、作ることはできる）。
    pub fn listed(self) -> bool {
        LISTED.contains(&self.to_u32())
    }

    /// 簡略に描いている閉じ方を含むか。
    pub fn simplified(self) -> bool {
        matches!(self.bottom, Closure::Snap | Closure::Auto)
    }
}

/// よく使われる型（はじめに一覧へ出す）。
pub const COMMON: [u32; 6] = [20200301, 20200101, 10200301, 55200301, 60200001, 20210101];

/// 公開の刀型の一覧に載っている A 類の番号（130 型）。
pub const LISTED: [u32; 130] = [1010000, 1100001, 1100003, 1110001, 1110003, 1200001, 1200003, 1210001, 1210003, 1400001, 1400003, 10010100, 10010300, 10100101, 10100103, 10100301, 10100303, 10110101, 10110103, 10110301, 10110303, 10200101, 10200103, 10200301, 10200303, 10210101, 10210103, 10210301, 10210303, 10400101, 10400103, 10400301, 10400303, 11010100, 11010300, 11100101, 11100103, 11100301, 11100303, 11110103, 11110301, 11200101, 11200103, 11200301, 11200303, 11210101, 11210103, 11210301, 11210303, 11400101, 11400103, 11400301, 11400303, 20010100, 20010300, 20100101, 20100103, 20100301, 20100303, 20110101, 20110103, 20110301, 20110303, 20200101, 20200103, 20200301, 20200303, 20210101, 20210103, 20210301, 20210303, 20400101, 20400103, 20400301, 20400303, 40010100, 40010300, 40100101, 40100103, 40100301, 40100303, 40110101, 40110103, 40110301, 40110303, 40200101, 40200103, 40200301, 40200303, 40210101, 40210103, 40210301, 40210303, 40400101, 40400103, 40400301, 40400303, 55010100, 55010300, 55100101, 55100103, 55100301, 55100303, 55110101, 55110103, 55110301, 55110303, 55200101, 55200103, 55200301, 55200303, 55210101, 55210103, 55210301, 55210303, 55400101, 55400103, 55400301, 55400303, 60010000, 60100001, 60100003, 60110001, 60110003, 60200001, 60200003, 60210001, 60210003, 60400001, 60400003];

/// のりしろの幅と、面の並び（左端から、のりしろ、背面、左側面、正面、右側面）。
struct Body {
    x: [f32; 5],
    l: f32,
    w: f32,
    h: f32,
    t: f32,
}

/// 描いたもの: パネル（多角形）、折り線、穴（切り線の輪）。
#[derive(Default)]
struct Sketch {
    panels: Vec<Vec<P2>>,
    creases: Vec<(P2, P2)>,
    holes: Vec<Vec<P2>>,
    /// 名前を付けるための、パネルの中の点と名前。
    names: Vec<(P2, &'static str)>,
}

impl Sketch {
    fn panel(&mut self, points: Vec<P2>, name: &'static str) {
        let n = points.len() as f32;
        let mut c = points.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
        c = [c[0] / n, c[1] / n];
        self.names.push((c, name));
        self.panels.push(points);
    }
}

/// 端（上か底）の描き方。`s` は面の端から外へ離れる向きの距離。
struct End<'a> {
    body: &'a Body,
    top: bool,
    /// 面の端の高さ（上なら面の高さ、底なら 0）。
    y0: f32,
    /// 面ごとの、端を内へ下げる量（下に入るフラップの面ほど、紙の厚みぶん低くする）。
    drop: [f32; 4],
}

impl<'a> End<'a> {
    /// 面 `i` に付くものを描くための端（その面の端の高さから）。
    fn wall(&self, i: usize) -> End<'a> {
        let y0 = if self.top { self.y0 - self.drop[i] } else { self.y0 + self.drop[i] };
        End { body: self.body, top: self.top, y0, drop: [0.0; 4] }
    }
    fn p(&self, x: f32, s: f32) -> P2 {
        if self.top { [x, self.y0 + s] } else { [x, self.y0 - s] }
    }
    fn names(&self) -> EndNames {
        if self.top {
            EndNames { lid: "上のフタ", tuck: "上の差し込み", left: "上の左フラップ", right: "上の右フラップ", flap: "上のフラップ" }
        } else {
            EndNames { lid: "底のフタ", tuck: "底の差し込み", left: "底の左フラップ", right: "底の右フラップ", flap: "底のフラップ" }
        }
    }
}

struct EndNames {
    lid: &'static str,
    tuck: &'static str,
    left: &'static str,
    right: &'static str,
    flap: &'static str,
}

/// 面の番号（0 背面、1 左側面、2 正面、3 右側面）の左右の端。
fn span(b: &Body, i: usize) -> (f32, f32) {
    (b.x[i], b.x[i + 1])
}

/// 台形のフラップ（根は x0..x1、深さ d、先は左右を `slant` ずつ細くする）。
fn flap(e: &End, x0: f32, x1: f32, d: f32, slant: [f32; 2]) -> Vec<P2> {
    vec![e.p(x0, 0.0), e.p(x1, 0.0), e.p(x1 - slant[1], d), e.p(x0 + slant[0], d)]
}

/// 根で折る線。
fn crease(sketch: &mut Sketch, e: &End, x0: f32, x1: f32, s: f32) {
    sketch.creases.push((e.p(x0, s), e.p(x1, s)));
}

/// 差し込みの長さ。
fn tuck_len(b: &Body) -> f32 {
    (0.2 * b.w + 6.0).min(25.0).min(0.5 * b.h).max(4.0)
}

/// 左右のフラップ（差し込みのフタの下に入る）。側面（1 と 3）に付ける。
fn dust_flaps(sketch: &mut Sketch, e: &End) {
    let b = e.body;
    let names = e.names();
    for (i, name) in [(1, names.left), (3, names.right)] {
        let e = &e.wall(i);
        let (x0, x1) = span(b, i);
        let (x0, x1) = (x0 + 1.0, x1 - 1.0);
        let d = (0.4 * b.l).min(0.5 * b.l - 1.0).max(3.0);
        let slant = d.min(0.35 * (x1 - x0));
        sketch.panel(flap(e, x0, x1, d, [slant, slant]), name);
        crease(sketch, e, x0, x1, 0.0);
    }
}

/// フタ（面 `k` に付く）。`tuck` なら先に差し込みを付ける。返り値はフタの左右の端。
fn lid(sketch: &mut Sketch, e: &End, k: usize, tuck: bool) -> (f32, f32) {
    let e = &e.wall(k);
    let b = e.body;
    let names = e.names();
    let (x0, x1) = span(b, k);
    let depth = b.w;
    sketch.panel(vec![e.p(x0, 0.0), e.p(x1, 0.0), e.p(x1, depth), e.p(x0, depth)], names.lid);
    crease(sketch, e, x0, x1, 0.0);
    if tuck {
        let tl = tuck_len(b);
        let ti = (0.5 * b.t).max(0.5);
        let c = (3.0f32).min(tl / 3.0);
        let (a0, a1) = (x0 + ti, x1 - ti);
        sketch.panel(vec![e.p(a0, depth), e.p(a1, depth), e.p(a1, depth + tl - c), e.p(a1 - c, depth + tl), e.p(a0 + c, depth + tl), e.p(a0, depth + tl - c)], names.tuck);
        crease(sketch, e, a0, a1, depth);
    }
    (x0, x1)
}

fn draw_end(sketch: &mut Sketch, e: &End, closure: Closure, front: bool) {
    let b = e.body;
    let names = e.names();
    let k = if front { 2 } else { 0 };
    let far = 2 - k;
    match closure {
        Closure::Open => {}
        Closure::Tuck => {
            lid(sketch, e, k, true);
            dust_flaps(sketch, e);
        }
        Closure::Hanger => {
            // 番号の面は、吊り下げを伸ばす面。フタは、その向かいの面に付く。
            lid(sketch, e, far, true);
            dust_flaps(sketch, e);
            // 面を上へ伸ばす（折らない）。穴は、横長の角の丸い穴。
            let e = &e.wall(k);
            let (x0, x1) = span(b, k);
            let hh = (0.25 * b.h).clamp(30.0, 50.0);
            sketch.panel(vec![e.p(x0, 0.0), e.p(x1, 0.0), e.p(x1, hh), e.p(x0, hh)], if k == 0 { "背面" } else { "正面" });
            let (cx, cy) = (0.5 * (x0 + x1), 0.6 * hh);
            let half = (0.2 * b.l).min(15.0);
            let r = 4.0f32.min(0.15 * hh);
            let mut ring = Vec::new();
            for (centre, from) in [(cx + half - r, -90.0f32), (cx - half + r, 90.0)] {
                for k in 0..=8 {
                    let a = (from + k as f32 * 22.5).to_radians();
                    ring.push(e.p(centre + r * a.cos(), cy + r * a.sin()));
                }
            }
            sketch.holes.push(ring);
        }
        Closure::SideTuck => {
            let (x0, x1) = lid(sketch, e, k, false);
            let (ek, ef) = (e.wall(k), e.wall(far));
            // フタの左右の翼。根の両端を少しあけ、先の角を落とす。
            let clear = (1.5 * b.t).max(1.0);
            let root = (clear, b.w - clear);
            // 翼は差し込みと同じく、根の長さの半分より短くする（フタを開けられる、差し込みのフタとして扱われる）。
            let dw = (0.5 * b.h).min(0.45 * (root.1 - root.0)).clamp(3.0, 30.0);
            let cut = (0.3 * dw).min(0.25 * (root.1 - root.0));
            for (edge, dir, name) in [(x1, 1.0f32, names.right), (x0, -1.0, names.left)] {
                let pts = vec![ek.p(edge, root.0), ek.p(edge + dir * dw, root.0 + cut), ek.p(edge + dir * dw, root.1 - cut), ek.p(edge, root.1)];
                sketch.panel(pts, name);
                sketch.creases.push((ek.p(edge, root.0), ek.p(edge, root.1)));
            }
            // 向かいの面の内フラップ（両側を 10 度ずつ細く）。
            let (f0, f1) = span(b, far);
            let (f0, f1) = (f0 + 1.0, f1 - 1.0);
            let d = (b.w - clear).max(3.0);
            let sl = (d * 10f32.to_radians().tan()).min(0.3 * (f1 - f0));
            sketch.panel(flap(&ef, f0, f1, d, [sl, sl]), names.flap);
            crease(sketch, &ef, f0, f1, 0.0);
        }
        Closure::Flaps | Closure::Reduced => {
            let slot = (2.0 * b.t).max(1.0);
            for i in 0..4 {
                let e = &e.wall(i);
                let (x0, x1) = span(b, i);
                let (x0, x1) = (x0 + 0.5 * slot, x1 - 0.5 * slot);
                let long = i % 2 == 0;
                let depth = match (closure, long) {
                    (_, false) => 0.5 * b.w,
                    (Closure::Flaps, true) => b.w,
                    (_, true) if i == k => b.w,
                    _ => 0.5 * b.w,
                };
                let name = match i {
                    1 => names.left,
                    3 => names.right,
                    _ if i == k => names.lid,
                    _ => names.flap,
                };
                sketch.panel(flap(e, x0, x1, depth, [0.0, 0.0]), name);
                crease(sketch, e, x0, x1, 0.0);
            }
        }
        Closure::Snap => {
            // 1-2-3 底（簡略）: フタの面は、まん中に舌の出たフラップ。向かいは半分の深さ。左右は台形。
            let (x0, x1) = span(b, k);
            let half = 0.5 * b.w;
            let tongue = (0.35 * b.l).min(x1 - x0 - 4.0);
            let tx = 0.5 * (x0 + x1);
            let td = (half + 0.3 * b.w).min(b.w - 1.0);
            sketch.panel(vec![e.p(x0, 0.0), e.p(x1, 0.0), e.p(x1, half), e.p(tx + 0.5 * tongue, half), e.p(tx + 0.5 * tongue - 3.0, td), e.p(tx - 0.5 * tongue + 3.0, td), e.p(tx - 0.5 * tongue, half), e.p(x0, half)], names.lid);
            crease(sketch, e, x0, x1, 0.0);
            let (f0, f1) = span(b, far);
            let (f0, f1) = (f0 + 0.5, f1 - 0.5);
            sketch.panel(flap(e, f0, f1, half, [0.0, 0.0]), names.flap);
            crease(sketch, e, f0, f1, 0.0);
            for (i, name) in [(1, names.left), (3, names.right)] {
                let (s0, s1) = span(b, i);
                let (s0, s1) = (s0 + 0.5, s1 - 0.5);
                let d = (0.45 * b.l).min(1.2 * (s1 - s0)).max(3.0);
                let slant = (0.45 * (s1 - s0)).min(d);
                sketch.panel(flap(e, s0, s1, d, [slant, slant]), name);
                crease(sketch, e, s0, s1, 0.0);
            }
        }
        Closure::Auto => {
            // ワンタッチ底（簡略）: 4 枚とも、片側を斜めに落としたフラップ。
            for i in 0..4 {
                let (x0, x1) = span(b, i);
                let (x0, x1) = (x0 + 0.5, x1 - 0.5);
                let long = i % 2 == 0;
                let d = if long { (0.5 * b.w + 0.15 * b.w).min(b.w - 1.0) } else { 0.5 * b.w };
                let slant = d.min(0.6 * (x1 - x0));
                let name = match i {
                    0 => "背面のフラップ",
                    1 => names.left,
                    2 => "正面のフラップ",
                    _ => names.right,
                };
                sketch.panel(flap(e, x0, x1, d, [0.0, slant]), name);
                crease(sketch, e, x0, x1, 0.0);
            }
        }
    }
}

/// 端で、面ごとに縁を内へ下げる量。閉じたとき、下に入るフラップの面ほど、紙の厚みぶん低くする
/// （同じ高さのままだと、重なったフラップどうしが同じ平面に来て、下のものが上のフタを突き抜けて見える）。
/// 組み込みの差し込み箱と同じく、型の線では、面の端に紙の厚みぶんの小さな段ができる。
fn drops(closure: Closure, front: bool, t: f32) -> [f32; 4] {
    let k = if front { 2 } else { 0 };
    let far = 2 - k;
    let mut d = [0.0; 4];
    match closure {
        // 左右のフラップが、フタの下に入る。
        Closure::Tuck | Closure::Hanger => {
            d[1] = t;
            d[3] = t;
        }
        // 向かいの面の内フラップが、フタの下に入る。
        Closure::SideTuck => d[far] = t,
        // 左右のフラップがいちばん下、その上に向かいの面のフラップ、いちばん上に外のフラップ。
        Closure::Flaps | Closure::Reduced => {
            d[1] = 2.0 * t;
            d[3] = 2.0 * t;
            d[far] = t;
        }
        Closure::Open | Closure::Snap | Closure::Auto => {}
    }
    d
}

/// 番号と内寸（l 正面の幅、w 奥行き、h 高さ、t 紙の厚み。mm）から、型の線を描く。
/// 返り値は、線と、正面の中の点と、パネルの名前の手がかりと、穴（中心と半分の幅・高さ）。
fn sketch(code: Code, l: f32, w: f32, h: f32, t: f32) -> (Sketch, Body) {
    let glue = (0.8 * (w - t)).clamp(4.0, 15.0);
    let closed = |c: Closure| if c == Closure::Open { 0.0 } else { 1.0 };
    let hw = h + t * (closed(code.top) + closed(code.bottom));
    let mut x = [glue; 5];
    // のりしろの付く背面は、紙 1 枚ぶん狭くする。筒を閉じたとき、のりしろが右側面の内側（紙 1 枚ぶん内）に来る
    // （同じ幅のままだと、のりしろが右側面と同じ平面に来て、外へ突き抜けて見える。組み込みの差し込み箱と同じ）。
    for (i, width) in [l - t, w, l, w].into_iter().enumerate() {
        x[i + 1] = x[i] + width;
    }
    let body = Body { x, l, w, h: hw, t };
    let (top, bottom) = (drops(code.top, code.top_front, t), drops(code.bottom, code.bottom_front, t));
    let mut sk = Sketch::default();
    for (i, name) in ["背面", "左側面", "正面", "右側面"].into_iter().enumerate() {
        let (x0, x1) = span(&body, i);
        let (y0, y1) = (bottom[i], hw - top[i]);
        sk.panel(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], name);
        if i > 0 {
            sk.creases.push(([x0, 0.0], [x0, hw]));
        }
    }
    // のりしろ（背面の左。角を落とす）。
    let bev = (5.0f32).min(hw / 4.0);
    let (g0, g1) = (bottom[0], hw - top[0]);
    sk.panel(vec![[0.0, g0 + bev], [glue, g0], [glue, g1], [0.0, g1 - bev]], "のりしろ");
    sk.creases.push(([glue, 0.0], [glue, hw]));
    draw_end(&mut sk, &End { body: &body, top: true, y0: hw, drop: top }, code.top, code.top_front);
    draw_end(&mut sk, &End { body: &body, top: false, y0: 0.0, drop: bottom }, code.bottom, code.bottom_front);
    (sk, body)
}

/// 点 `p` が線分 a-b の上（端を除く）にあれば、a からの割合。
fn on_segment(p: P2, a: P2, b: P2) -> Option<f32> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    if l2 < 1e-10 {
        return None;
    }
    let s = ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2;
    let q = [a[0] + d[0] * s, a[1] + d[1] * s];
    let off = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
    (off < 1e-3 && s > 1e-5 && s < 1.0 - 1e-5).then_some(s)
}

/// 点を 1/1000 mm に丸めた、比べるための値。
type Key = (i64, i64);

/// パネルの辺から、切り線と折り線を作る。
/// 辺を、ほかのパネルの角で細かく分け、1 つのパネルにしか使われない部分が切り線。
/// 2 つのパネルが共有する部分は、折り線の上なら折り線、そうでなければ線なし（つながった 1 枚の紙）。
fn lines(sk: &Sketch) -> Vec<(P2, P2, LineKind)> {
    let corners: Vec<P2> = sk.panels.iter().flatten().copied().collect();
    let key = |p: P2| ((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64);
    let mut count: std::collections::BTreeMap<(Key, Key), (P2, P2, usize)> = std::collections::BTreeMap::new();
    for poly in &sk.panels {
        for k in 0..poly.len() {
            let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
            let mut cuts: Vec<f32> = corners.iter().filter_map(|&c| on_segment(c, a, b)).collect();
            cuts.push(0.0);
            cuts.push(1.0);
            cuts.sort_by(f32::total_cmp);
            cuts.dedup_by(|x, y| (*x - *y).abs() < 1e-6);
            for pair in cuts.windows(2) {
                let at = |s: f32| [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s];
                let (p, q) = (at(pair[0]), at(pair[1]));
                let (kp, kq) = (key(p), key(q));
                let id = if kp <= kq { (kp, kq) } else { (kq, kp) };
                count.entry(id).or_insert((p, q, 0)).2 += 1;
            }
        }
    }
    let on_crease = |p: P2, q: P2| {
        let m = [0.5 * (p[0] + q[0]), 0.5 * (p[1] + q[1])];
        sk.creases.iter().any(|&(a, b)| on_segment(m, a, b).is_some())
    };
    let mut out = Vec::new();
    for (p, q, n) in count.into_values() {
        if n == 1 {
            out.push((p, q, LineKind::Cut));
        } else if on_crease(p, q) {
            out.push((p, q, LineKind::Crease));
        }
    }
    for ring in &sk.holes {
        for k in 0..ring.len() {
            out.push((ring[k], ring[(k + 1) % ring.len()], LineKind::Cut));
        }
    }
    out
}

/// 番号と内寸から、型の線（DXF を読んだときと同じ形）を作る。
pub fn drawing(code: Code, l: f32, w: f32, h: f32, t: f32) -> Drawing {
    let (sk, _) = sketch(code, l, w, h, t);
    to_drawing(&lines(&sk))
}

fn to_drawing(lines: &[(P2, P2, LineKind)]) -> Drawing {
    let groups = vec![
        Group { name: "CUT".into(), color: 1, count: lines.iter().filter(|l| l.2 == LineKind::Cut).count(), guess: LineKind::Cut },
        Group { name: "CREASE".into(), color: 3, count: lines.iter().filter(|l| l.2 == LineKind::Crease).count(), guess: LineKind::Crease },
    ];
    let paths = lines.iter().map(|&(a, b, kind)| Path { group: if kind == LineKind::Cut { 0 } else { 1 }, points: vec![a, b] }).collect();
    Drawing { groups, paths, unit_mm: Some(1.0), skipped: 0 }
}

impl Template {
    /// ECMA の番号と内寸（l 正面の幅、w 奥行き、h 高さ。mm）から、型紙を作る。
    pub fn ecma(code: Code, l: f32, w: f32, h: f32, t: f32) -> Result<Template, String> {
        let (sk, body) = sketch(code, l, w, h, t);
        let drawing = to_drawing(&lines(&sk));
        let settings = ImportSettings::guess(&drawing);
        let front = [0.5 * (body.x[2] + body.x[3]), 0.5 * body.h];
        let (mut tpl, _) = Template::imported_rooted(&drawing, &settings, t, Some(front))?;
        // 紙の座標は、線の左下を原点にしている。
        let (min, _) = drawing.bounds();
        let place = |p: P2| [p[0] - min[0], p[1] - min[1]];
        // パネルの名前: 描いたときの名前を、そのパネルの中の点で当てる（当たらなければ、読み込みの名前のまま）。
        for panel in tpl.panels.iter_mut() {
            if let Some(&(_, name)) = sk.names.iter().rev().find(|(c, _)| crate::template::inside(&panel.outline, place(*c))) {
                panel.name = name;
            }
        }
        // 吊り下げの穴。
        for ring in &sk.holes {
            let (lo, hi) = ring.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
            let c = place([0.5 * (lo[0] + hi[0]), 0.5 * (lo[1] + hi[1])]);
            tpl.holes.push([c[0], c[1], 0.5 * (hi[0] - lo[0]), 0.5 * (hi[1] - lo[1])]);
        }
        Ok(tpl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_round_trip() {
        for n in LISTED {
            let code = Code::from_u32(n).unwrap_or_else(|| panic!("{n}"));
            assert_eq!(code.to_u32(), n);
            assert_eq!(Code::parse(&code.name()), Some(code));
        }
        assert_eq!(Code::parse("A20.20.03.01").map(|c| c.to_u32()), Some(20200301));
        assert_eq!(Code::parse("A60.20.03.01"), None);
    }

    #[test]
    fn every_listed_code_folds() {
        for n in LISTED {
            let code = Code::from_u32(n).unwrap();
            let tpl = Template::ecma(code, 100.0, 60.0, 150.0, 0.4).unwrap_or_else(|e| panic!("{}: {e}", code.name()));
            let names: Vec<&str> = tpl.panels.iter().map(|p| p.name).collect();
            for wall in ["正面", "背面", "左側面", "右側面", "のりしろ"] {
                assert_eq!(names.iter().filter(|&&n| n == wall).count(), 1, "{}: {names:?}", code.name());
            }
            assert_eq!(tpl.panels[0].name, "正面", "{}", code.name());
        }
    }
}

#[cfg(test)]
mod pictures {
    use super::*;

    /// 線を SVG に書き出す（目で確かめる用。`cargo test -p pakenoba-core ecma_svg -- --ignored`）。
    #[test]
    #[ignore]
    fn ecma_svg() {
        let dir = std::env::var("ECMA_SVG").unwrap_or_else(|_| "/tmp/ecma".into());
        std::fs::create_dir_all(&dir).unwrap();
        for n in [20200301, 20210101, 10100103, 11110301, 40400103, 55200301, 60200001, 1400001, 55210103] {
            let code = Code::from_u32(n).unwrap();
            let d = drawing(code, 100.0, 60.0, 150.0, 0.4);
            let (lo, hi) = d.bounds();
            let mut svg = format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='{} {} {} {}' width='{}' height='{}'><rect x='{}' y='{}' width='100%' height='100%' fill='white'/>", lo[0] - 5.0, -hi[1] - 5.0, hi[0] - lo[0] + 10.0, hi[1] - lo[1] + 10.0, (hi[0] - lo[0] + 10.0) * 2.0, (hi[1] - lo[1] + 10.0) * 2.0, lo[0] - 5.0, -hi[1] - 5.0);
            for p in &d.paths {
                let color = if p.group == 0 { "black" } else { "red" };
                let dash = if p.group == 0 { "" } else { " stroke-dasharray='2 1'" };
                svg += &format!("<line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{color}' stroke-width='0.4'{dash}/>", p.points[0][0], -p.points[0][1], p.points[1][0], -p.points[1][1]);
            }
            svg += &format!("<text x='{}' y='{}' font-size='8'>{}</text></svg>", lo[0], -hi[1] + 4.0, code.name());
            std::fs::write(format!("{dir}/{}.svg", code.name()), svg).unwrap();
        }
    }
}
