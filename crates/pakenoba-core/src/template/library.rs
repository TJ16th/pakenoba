//! 型（寸法から型紙を作る関数）。
//!
//! 状態:
//! - `tuck_box`（差し込み箱）、`prism`（角柱）、`pillow`（ピロー型）、`gable`（ゲーブル型）: 検査（`crate::check`）を通る。
//!   ゲーブル型は、持ち手の留めの形がまだ型紙に入っていない。
//! - `carry_bag`（丸みのある手提げ型）: 試作から移しただけの下書き。開け口が決まっておらず、上の端も閉じていない。
//!   直すまでは、画面の型の一覧に出さない。

use super::*;

/// 差し込み箱や角柱のフタを、お客さんが開けきったときに、閉じた状態から起こす角度。
/// 垂直（90 度）より少し先まで倒れる。
const LID_LIFT: f32 = 110.0 * std::f32::consts::PI / 180.0;

/// フタの下の内側のフラップを、お客さんが開けきったときに、閉じた状態から起こす角度。
/// 垂直より少し外へ開く。フタの倒れ（垂直から 20 度）より小さくしておくと、フタにぶつからない。
const FLAP_LIFT: f32 = 100.0 * std::f32::consts::PI / 180.0;

/// 差し込み箱のフラップの長さ（mm）。`None` は、寸法から自動で決める。
///
/// 範囲の外の値は、作るときに範囲の中へ寄せる（[`TuckFlaps::ranges`]）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TuckFlaps {
    /// のりしろの幅。
    pub glue: Option<f32>,
    /// 左右のフラップ（差し込みの下で、開け口をふさぐ小さいフタ）の長さ。
    pub dust: Option<f32>,
    /// 差し込み（ベロ）の長さ。
    pub tuck: Option<f32>,
}

impl TuckFlaps {
    /// 自動のときの長さ（のりしろ、左右のフラップ、差し込み）。
    pub fn auto(w: f32, d: f32, h: f32, t: f32) -> [f32; 3] {
        [(0.8 * (d - t)).clamp(4.0, 15.0), (0.5 * w - 1.0).min(0.6 * d).max(3.0), (0.3 * h).clamp(5.0, 18.0)]
    }

    /// 選べる長さの範囲（のりしろ、左右のフラップ、差し込み）。
    /// - のりしろは、背面の幅の半分まで（40 mm まで）。
    /// - 左右のフラップは、開け口の幅の半分まで（左右がまん中でぶつからない）。
    /// - 差し込みは、正面の高さの 8 割まで（正面の内側に入りきる）。
    pub fn ranges(w: f32, _d: f32, h: f32, _t: f32) -> [std::ops::RangeInclusive<f32>; 3] {
        [4.0..=(0.5 * w).clamp(4.0, 40.0), 3.0..=(0.5 * w - 1.0).max(3.0), 3.0..=(0.8 * h).max(3.0)]
    }

    /// 作るときの長さ（のりしろ、左右のフラップ、差し込み）。決めていないものは自動、範囲の外は範囲の中へ。
    pub fn resolve(&self, w: f32, d: f32, h: f32, t: f32) -> [f32; 3] {
        let auto = TuckFlaps::auto(w, d, h, t);
        let ranges = TuckFlaps::ranges(w, d, h, t);
        let chosen = [self.glue, self.dust, self.tuck];
        std::array::from_fn(|i| chosen[i].filter(|v| v.is_finite()).unwrap_or(auto[i]).clamp(*ranges[i].start(), *ranges[i].end()))
    }
}

/// 持ち手つきピロー型の持ち手（正面から見て等脚台形）の、下の幅、上の幅、高さ（mm）。`None` は、寸法から自動で決める。
///
/// 範囲の外の値は、作るときに範囲の中へ寄せる（[`HandleSize::ranges`]）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HandleSize {
    pub bottom: Option<f32>,
    pub top: Option<f32>,
    pub height: Option<f32>,
}

impl HandleSize {
    /// 自動のときの、下の幅、上の幅、高さ（`len` は横の長さ、`height` は 1 枚の面の高さ）。
    pub fn auto(len: f32, height: f32) -> [f32; 3] {
        let bottom = 0.46 * len;
        [bottom, bottom * 0.82, (0.35 * height).clamp(25.0, 50.0)]
    }

    /// 選べる範囲。幅は、横の長さいっぱいまで。上の幅は、下より広くもできる（逆さの台形）。
    pub fn ranges(len: f32, height: f32) -> [std::ops::RangeInclusive<f32>; 3] {
        let wide = len.max(30.0);
        [20.0..=wide, 15.0..=wide, 15.0..=(1.2 * height).max(60.0)]
    }

    /// 作るときの値。決めていないものは自動、範囲の外は範囲の中へ。
    pub fn resolve(&self, len: f32, height: f32) -> [f32; 3] {
        let auto = HandleSize::auto(len, height);
        let ranges = HandleSize::ranges(len, height);
        let chosen = [self.bottom, self.top, self.height];
        std::array::from_fn(|i| chosen[i].filter(|v| v.is_finite()).unwrap_or(auto[i]).clamp(*ranges[i].start(), *ranges[i].end()))
    }
}

impl Template {
    /// 差し込み式の箱（キャラメル箱）。寸法は内寸（mm）。
    ///
    /// 厚みの扱いは単純なルールで、重なる部分を紙 1 枚ぶんずらす。
    /// - のりしろは背面の内側に入る。そのぶん左側面を 1 枚ぶん短く、背面を 1 枚ぶん長くする。
    /// - フタは差し込みフラップ（左右の小さいフタ）の上に乗る。そのぶん正面と背面を上下に 1 枚ぶんずつ高くする。
    /// - 差し込み（ベロ）は正面の内側に入る。そのぶんフタを 1 枚ぶん短くする。
    pub fn tuck_box(w: f32, d: f32, h: f32, t: f32) -> Template {
        Template::tuck_box_with(w, d, h, t, TuckFlaps::default())
    }

    /// 差し込み式の箱。フラップの長さを `flaps` で決める（決めていないものは自動）。
    pub fn tuck_box_with(w: f32, d: f32, h: f32, t: f32, flaps: TuckFlaps) -> Template {
        let [glue, dust, tuck] = flaps.resolve(w, d, h, t);
        let tuck_taper = (0.15 * w).min(tuck * 0.6);

        let u_l = glue;
        let u_f = u_l + (d - t);
        let u_r = u_f + w;
        let u_b = u_r + d;
        let u_end = u_b + w + t;
        let quarter = std::f32::consts::FRAC_PI_2;

        const WALLS: (f32, f32) = (0.0, 0.40);
        let mut panels = vec![
            Panel::flat("正面", rect(u_f, -t, u_r, h + t), None, Role::Face),
            Panel::flat("左側面", rect(u_l, 0.0, u_f, h), Some(Hinge { parent: 0, a: [u_f, 0.0], b: [u_f, h], angle: quarter, window: WALLS, motion: Motion::Fixed }), Role::Face),
            Panel::flat("右側面", rect(u_r, 0.0, u_b, h), Some(Hinge { parent: 0, a: [u_r, 0.0], b: [u_r, h], angle: quarter, window: WALLS, motion: Motion::Fixed }), Role::Face),
            Panel::flat("背面", rect(u_b, -t, u_end, h + t), Some(Hinge { parent: 2, a: [u_b, 0.0], b: [u_b, h], angle: quarter, window: WALLS, motion: Motion::Fixed }), Role::Face),
        ];
        let c = 2.0f32.min(h * 0.1);
        let k = glue * 0.35;
        panels.push(Panel::flat(
            "のりしろ",
            ccw(vec![[0.0, c + k], [u_l, c], [u_l, h - c], [0.0, h - c - k]]),
            Some(Hinge { parent: 1, a: [u_l, c], b: [u_l, h - c], angle: quarter, window: WALLS, motion: Motion::Fixed }),
            Role::GlueTab,
        ));

        // 上と下は同じ形の鏡写し。sign = 1 が上、-1 が下。
        for (sign, names, windows) in [
            (-1.0f32, ["底の左フラップ", "底の右フラップ", "底のフタ", "底の差し込み"], [(0.40, 0.54), (0.46, 0.60), (0.54, 0.70)]),
            (1.0f32, ["上の左フラップ", "上の右フラップ", "上のフタ", "上の差し込み"], [(0.70, 0.82), (0.74, 0.88), (0.84, 1.0)]),
        ] {
            let first = panels.len();
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
                    // フタを起こしたあとで、左右のフラップを外へ起こす。
                    Some(Hinge { parent, a: [u0, side_edge], b: [u1, side_edge], angle: quarter, window: dust_window, motion: Motion::Flap(FLAP_LIFT) }),
                    Role::Flap,
                ));
            }

            // フタ。背面につながり、奥行きより紙 1 枚ぶん短い。
            let lid_far = lid_edge + sign * (d - t);
            let lid_index = panels.len();
            // 上も底も、同じ作りで、同じように開く。差し込みは、正面の内側に沿って抜ける。
            let clear = (tuck / (d - t)).min(1.0).asin();
            let (lid_motion, tuck_motion) = (Motion::Lift(LID_LIFT), Motion::Slide { lift: LID_LIFT, clear });
            panels.push(Panel::flat(names[2], rect(u_b, lid_edge, u_b + w, lid_far), Some(Hinge { parent: 3, a: [u_b, lid_edge], b: [u_b + w, lid_edge], angle: quarter, window: lid_window, motion: lid_motion }), Role::Face));

            // 差し込み（ベロ）。フタの先につながり、正面の内側に入る。
            let (s0, s1) = (u_b + 0.5, u_b + w - 0.5);
            panels.push(Panel::flat(
                names[3],
                ccw(vec![[s0, lid_far], [s1, lid_far], [s1 - tuck_taper, lid_far + sign * tuck], [s0 + tuck_taper, lid_far + sign * tuck]]),
                Some(Hinge { parent: lid_index, a: [s0, lid_far], b: [s1, lid_far], angle: quarter, window: tuck_window, motion: tuck_motion }),
                Role::Tuck,
            ));
            // 底のパネルは、底の開け閉め（開け口 1）で動く。
            if sign < 0.0 {
                panels[first..].iter_mut().for_each(|panel| panel.opening = 1);
            }
        }

        let (min, max) = bounds(&panels);
        let r = w.min(d).min(h) * 0.34;
        let marks = vec![([u_r, h * 0.55], r), ([u_b + w * 0.5, h + t], r * 0.8)];
        let openings = openings(&panels, &[("上のフタ", Some("上の差し込み"), Hold::Nothing), ("底のフタ", Some("底の差し込み"), Hold::Nothing)]);
        let glued = glued(&panels, &[("のりしろ", "背面")]);
        Template { panels, thickness: t, min, max, world: upright_world([u_f + w * 0.5, h * 0.5], d, t), tilt: 0.0, tilt_window: (0.0, 0.5), marks, holes: Vec::new(), open_angle: 0.0, openings, glued, drawn: None }
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
        let inset = 0.2 * side;
        // フラップの高さ。隣どうしが中でぶつからない高さと、展開図の上でフタの斜めの辺に当たらない高さの、低いほう
        // （角の数が多いと、フタが隣の側面の上まで張り出す）。
        let below_lid = if exterior < 1.5 { 0.9 * inset * exterior.tan() } else { f32::INFINITY };
        let flap = (0.2 * side * (interior * 0.5).tan()).min(below_lid);

        let u0 = shift;
        let wall_start = |k: usize| u0 + k as f32 * side;
        let last_end = wall_start(n) - shift;
        const WALLS: (f32, f32) = (0.0, 0.50);

        let mut panels = vec![Panel::flat("側面 1", rect(u0 - shift, -t, u0 + side, h + t), None, Role::Face)];
        const NAMES: [&str; 12] = ["側面 1", "側面 2", "側面 3", "側面 4", "側面 5", "側面 6", "側面 7", "側面 8", "側面 9", "側面 10", "側面 11", "側面 12"];
        // フタの差し込みが入る側面。フタのつけ根の向かい側（角の数が奇数なら、向かいの角をはさむ 2 枚）。
        // この側面にはフラップを付けず、差し込みを受ける。フタの縁を隠すよう、上下に紙 1 枚ぶん高くする。
        let tuck_walls: Vec<usize> = if n.is_multiple_of(2) { vec![n / 2] } else { vec![(n - 1) / 2, n.div_ceil(2)] };
        for (k, &name) in NAMES.iter().enumerate().take(n).skip(1) {
            let end = if k == n - 1 { last_end } else { wall_start(k + 1) };
            let rise = if tuck_walls.contains(&k) { t } else { 0.0 };
            panels.push(Panel::flat(name, rect(wall_start(k), -rise, end, h + rise), Some(Hinge { parent: k - 1, a: [wall_start(k), 0.0], b: [wall_start(k), h], angle: exterior, window: WALLS, motion: Motion::Fixed }), Role::Face));
        }
        let c = 2.0f32.min(h * 0.1);
        let k = glue * 0.35;
        panels.push(Panel::flat(
            "のりしろ",
            ccw(vec![[last_end, c], [last_end + glue, c + k], [last_end + glue, h - c - k], [last_end, h - c]]),
            Some(Hinge { parent: n - 1, a: [last_end, c], b: [last_end, h - c], angle: exterior, window: WALLS, motion: Motion::Fixed }),
            Role::GlueTab,
        ));

        let tuck = (0.25 * side).clamp(5.0, 15.0);
        for (sign, flap_name, lid_name, tuck_name, flap_window, tuck_window, lid_window) in
            [(-1.0f32, "底のフラップ", "底のフタ", "底の差し込み", (0.50, 0.60), (0.56, 0.68), (0.62, 0.76)), (1.0f32, "上のフラップ", "上のフタ", "上の差し込み", (0.74, 0.84), (0.80, 0.92), (0.86, 1.0))]
        {
            let side_edge = if sign > 0.0 { h } else { 0.0 };
            let lid_edge = if sign > 0.0 { h + t } else { -t };
            let first = panels.len();
            for wall in (1..n).filter(|wall| !tuck_walls.contains(wall)) {
                let a = wall_start(wall) + 0.5;
                let b = (if wall == n - 1 { last_end } else { wall_start(wall + 1) }) - 0.5;
                panels.push(Panel::flat(
                    flap_name,
                    ccw(vec![[a, side_edge], [b, side_edge], [b - inset, side_edge + sign * flap], [a + inset, side_edge + sign * flap]]),
                    Some(Hinge { parent: wall, a: [a, side_edge], b: [b, side_edge], angle: quarter, window: flap_window, motion: Motion::Flap(FLAP_LIFT) }),
                    Role::Flap,
                ));
            }
            // フタは、最初の側面の端を一辺とする正多角形。
            let mut lid = vec![[u0, lid_edge], [u0 + side, lid_edge]];
            for i in 1..n - 1 {
                let prev = lid[lid.len() - 1];
                let angle = exterior * i as f32;
                lid.push([prev[0] + side * angle.cos(), prev[1] + sign * side * angle.sin()]);
            }
            // 差し込みは側面の内側に入る。そのぶん、差し込みの付く辺を紙 1 枚ぶん内側へ寄せる。
            let amount: Vec<f32> = (0..n).map(|i| if tuck_walls.contains(&i) { t } else { 0.0 }).collect();
            let lid = inset_edges(&lid, &amount, sign);
            let lid_index = panels.len();
            // 上も底も、同じ作りで、同じように開く。
            panels.push(Panel::flat(lid_name, ccw(lid.clone()), Some(Hinge { parent: 0, a: [u0, lid_edge], b: [u0 + side, lid_edge], angle: quarter, window: lid_window, motion: Motion::Lift(LID_LIFT) }), Role::Face));
            // 差し込み（ベロ）。フタの辺から外へ伸び、折って側面の内側に入る。
            for &i in &tuck_walls {
                let (a, b) = (lid[i], lid[(i + 1) % n]);
                let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
                // フタの外へ向かう向き。
                let out = [dir[1] * sign, -dir[0] * sign];
                let (a, b) = ([a[0] + dir[0] * 0.5, a[1] + dir[1] * 0.5], [b[0] - dir[0] * 0.5, b[1] - dir[1] * 0.5]);
                let taper = (len * 0.15).min(tuck * 0.6);
                let tip = |p: P2, along: f32| [p[0] + dir[0] * along + out[0] * tuck, p[1] + dir[1] * along + out[1] * tuck];
                // 差し込みが側面の内側から抜けきるのは、フタのつけ根にいちばん近い端が、差し込みの高さだけ上がったとき。
                let nearest = (a[1] - lid_edge).abs().min((b[1] - lid_edge).abs()).max(1e-3);
                let motion = Motion::Slide { lift: LID_LIFT, clear: (tuck / nearest).min(1.0).asin() };
                panels.push(Panel::flat(tuck_name, ccw(vec![a, b, tip(b, -taper), tip(a, taper)]), Some(Hinge { parent: lid_index, a, b, angle: quarter, window: tuck_window, motion }), Role::Tuck));
            }
            // 底のパネルは、底の開け閉め（開け口 1）で動く。
            if sign < 0.0 {
                panels[first..].iter_mut().for_each(|panel| panel.opening = 1);
            }
        }

        let (min, max) = bounds(&panels);
        let r = side.min(h) * 0.3;
        let marks = vec![([u0 + side, h * 0.55], r), ([u0 + side * 0.5, h + t], r * 0.8)];
        let depth = side / (std::f32::consts::PI / n as f32).tan();
        let openings = openings(&panels, &[("上のフタ", Some("上の差し込み"), Hold::Nothing), ("底のフタ", Some("底の差し込み"), Hold::Nothing)]);
        let glued = glued(&panels, &[("のりしろ", "側面 1")]);
        Template { panels, thickness: t, min, max, world: upright_world([u0 + side * 0.5, h * 0.5], depth, t), tilt: 0.0, tilt_window: (0.0, 0.5), marks, holes: Vec::new(), open_angle: 0.0, openings, glued, drawn: None }
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
        let bend = || if kappa > 1e-6 { Some(Bend { curvature: kappa, window: MAIN, line: None, neck: None }) } else { None };

        let c = w * 0.5;
        let half_handle = w * 0.28;
        let handle_height = (0.3 * l).clamp(30.0, 60.0);
        let top = d + l;

        let mut panels = vec![Panel::flat("底", rect(0.0, 0.0, w, d), None, Role::Face)];
        panels.push(Panel { name: "正面", outline: rect(0.0, d, w, top), hinge: Some(Hinge { parent: 0, a: [0.0, d], b: [w, d], angle: rise, window: MAIN, motion: Motion::Fixed }), bend: bend(), curved: None, slices: 0, role: Role::Face, opening: 0 });
        panels.push(Panel { name: "背面", outline: rect(0.0, -l, w, 0.0), hinge: Some(Hinge { parent: 0, a: [0.0, 0.0], b: [w, 0.0], angle: rise, window: MAIN, motion: Motion::Fixed }), bend: bend(), curved: None, slices: 0, role: Role::Face, opening: 0 });
        for (name, parent, edge, sign) in [("正面の持ち手", 1usize, top, 1.0f32), ("背面の持ち手", 2usize, -l, -1.0f32)] {
            panels.push(Panel::flat(
                name,
                ccw(vec![[c - half_handle, edge], [c + half_handle, edge], [c + half_handle * 0.82, edge + sign * handle_height], [c - half_handle * 0.82, edge + sign * handle_height]]),
                Some(Hinge { parent, a: [c - half_handle, edge], b: [c + half_handle, edge], angle: handle_fold, window: MAIN, motion: Motion::Fixed }),
                Role::Face,
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
            outline.extend((0..=steps).map(&profile).map(|(y, z)| [edge + sign * z, d * 0.5 - y]));
            outline.extend((0..=steps).rev().map(&profile).map(|(y, z)| [edge + sign * z, d * 0.5 + y]));
            panels.push(Panel::flat(name, ccw(outline), Some(Hinge { parent: 0, a: [edge, 0.0], b: [edge, d], angle: std::f32::consts::FRAC_PI_2, window: SIDES, motion: Motion::Fixed }), Role::Face));
        }

        let (min, max) = bounds(&panels);
        let r = w.min(l) * 0.22;
        let marks = vec![([c, d + l * 0.5], r), ([w, d + l * 0.3], r * 0.6)];
        let hole = |edge: f32, sign: f32| [c, edge + sign * handle_height * 0.55, half_handle * 0.5, handle_height * 0.2];
        // 展開図では印刷面を手前に向け、組み上がるにつれて 90 度倒して、底を水平に寝かせる。
        // 倒しきると、x が幅、y が高さ（底の内側が 0）、z が奥行き（正面が手前）になる。
        let world = upright_world([c, d * 0.5], 0.0, t);
        Template { panels, thickness: t, min, max, world, tilt: std::f32::consts::FRAC_PI_2, tilt_window: (0.0, 0.5), marks, holes: vec![hole(top, 1.0), hole(-l, -1.0)], open_angle: 0.0, openings: Vec::new(), glued: Vec::new(), drawn: None }
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
            Panel::flat("正面", rect(u_f, -t, u_r, h), None, Role::Face),
            Panel::flat("左側面", rect(u_l, 0.0, u_f, h), Some(Hinge { parent: 0, a: [u_f, 0.0], b: [u_f, h], angle: quarter, window: WALLS, motion: fixed }), Role::Face),
            Panel::flat("右側面", rect(u_r, 0.0, u_b, h), Some(Hinge { parent: 0, a: [u_r, 0.0], b: [u_r, h], angle: quarter, window: WALLS, motion: fixed }), Role::Face),
            Panel::flat("背面", rect(u_b, -t, u_end, h), Some(Hinge { parent: 2, a: [u_b, 0.0], b: [u_b, h], angle: quarter, window: WALLS, motion: fixed }), Role::Face),
        ];
        let c = 2.0f32.min(h * 0.1);
        let k = glue * 0.35;
        panels.push(Panel::flat(
            "のりしろ",
            ccw(vec![[0.0, c + k], [u_l, c], [u_l, h - c], [0.0, h - c - k]]),
            Some(Hinge { parent: 1, a: [u_l, c], b: [u_l, h - c], angle: quarter, window: WALLS, motion: fixed }),
            Role::GlueTab,
        ));

        // 底は差し込み箱と同じ。底の開け閉め（開け口 1）で、同じように開く。
        let first_bottom = panels.len();
        let clear = t + 0.5;
        for (name, parent, u0, u1) in [("底の左フラップ", 1usize, u_l, u_f - clear), ("底の右フラップ", 2usize, u_r + clear, u_b - 0.5)] {
            let taper = (u1 - u0) * 0.15;
            panels.push(Panel::flat(
                name,
                ccw(vec![[u0, 0.0], [u1, 0.0], [u1 - taper, -dust], [u0 + taper, -dust]]),
                Some(Hinge { parent, a: [u0, 0.0], b: [u1, 0.0], angle: quarter, window: (0.50, 0.70), motion: Motion::Flap(FLAP_LIFT) }),
                Role::Flap,
            ));
        }
        let lid_far = -t - (d - t);
        let bottom_lid = panels.len();
        panels.push(Panel::flat("底のフタ", rect(u_b, -t, u_b + w, lid_far), Some(Hinge { parent: 3, a: [u_b, -t], b: [u_b + w, -t], angle: quarter, window: (0.75, 1.0), motion: Motion::Lift(LID_LIFT) }), Role::Face));
        let (s0, s1) = (u_b + 0.5, u_b + w - 0.5);
        let slide = Motion::Slide { lift: LID_LIFT, clear: (tuck / (d - t)).min(1.0).asin() };
        panels.push(Panel::flat(
            "底の差し込み",
            ccw(vec![[s0, lid_far], [s1, lid_far], [s1 - tuck_taper, lid_far - tuck], [s0 + tuck_taper, lid_far - tuck]]),
            Some(Hinge { parent: bottom_lid, a: [s0, lid_far], b: [s1, lid_far], angle: quarter, window: (0.60, 0.80), motion: slide }),
            Role::Tuck,
        ));
        panels[first_bottom..].iter_mut().for_each(|panel| panel.opening = 1);

        // 屋根と持ち手。組み立てでは折らず、開け閉めで動く。
        let ridge = h + slope;
        let mut roofs = [0usize; 2];
        for (i, (roof_name, handle_name, wall, u0)) in [("正面の屋根", "正面の持ち手", 0usize, u_f), ("背面の屋根", "背面の持ち手", 3usize, u_b)].into_iter().enumerate() {
            roofs[i] = panels.len();
            panels.push(Panel::flat(roof_name, rect(u0, h, u0 + w, ridge), Some(Hinge { parent: wall, a: [u0, h], b: [u0 + w, h], angle: 0.0, window: WALLS, motion: Motion::Follow(1.0) }), Role::Face));
            panels.push(Panel::flat(
                handle_name,
                rect(u0, ridge, u0 + w, ridge + handle),
                Some(Hinge { parent: roofs[i], a: [u0, ridge], b: [u0 + w, ridge], angle: 0.0, window: WALLS, motion: Motion::Follow(-1.0) }),
                Role::Face,
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
                Role::Face,
            ));
            let center_index = panels.len() - 1;
            for (roof_index, edge, dir) in [(roofs[0], front_edge, front_dir), (roofs[1], back_edge, back_dir)] {
                // 左奥の三角形は、背面の屋根とは展開図の上で離れている（間に、のりしろの合わせ目がある）。
                // 左の三角に斜めの折り線でつなぎ、背面の屋根とは、のりしろで貼る。
                if wall == 1 && roof_index == roofs[1] {
                    let wing = panels.len();
                    panels.push(Panel::flat(
                        wing_name,
                        ccw(vec![[base0, h], [base0, ridge], [base0 + half, ridge]]),
                        Some(Hinge { parent: center_index, a: [base0, h], b: [base0 + half, ridge], angle: 0.0, window: WALLS, motion: Motion::GableDiagonal(ratio) }),
                        Role::Gusset,
                    ));
                    panels.push(Panel::flat(
                        "屋根ののりしろ",
                        ccw(vec![[0.0, h + c + k], [base0, h + c], [base0, ridge - c], [0.0, ridge - c - k]]),
                        Some(Hinge { parent: wing, a: [base0, h + c], b: [base0, ridge - c], angle: quarter, window: WALLS, motion: Motion::GableWing(ratio) }),
                        Role::GlueTab,
                    ));
                    continue;
                }
                panels.push(Panel::flat(
                    wing_name,
                    ccw(vec![[edge, h], [edge, ridge], [edge + dir * half, ridge]]),
                    Some(Hinge { parent: roof_index, a: [edge, h], b: [edge, ridge], angle: quarter, window: WALLS, motion: Motion::GableWing(ratio) }),
                    Role::Gusset,
                ));
            }
        }

        let (min, max) = bounds(&panels);
        let r = w.min(h) * 0.26;
        let marks = vec![([u_f + w * 0.5, h * 0.5], r), ([u_f + w * 0.22, h], r * 0.55)];
        let hole = |u0: f32| [u0 + w * 0.5, ridge + handle * 0.5, w * 0.14, handle * 0.24];
        // 屋根は、持ち手の留めで閉じた状態を保つ（留めの形は、まだ型紙に入っていない）。
        let openings = openings(&panels, &[("正面の屋根", None, Hold::Latch), ("底のフタ", Some("底の差し込み"), Hold::Nothing)]);
        let glued = glued(&panels, &[("のりしろ", "背面"), ("屋根ののりしろ", "背面の屋根")]);
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
            openings,
            glued,
            drawn: None,
        }
    }

    /// 持ち手つきピロー型（上下が対称なもの）。[`Template::pillow_with`] の、持ち手あり・首なし。
    pub fn pillow(len: f32, height: f32, t: f32, bulge: f32) -> Template {
        Template::pillow_with(len, height, t, bulge, Some(HandleSize::default()), false)
    }

    /// ピロー型。2 枚の面が下の折り線でつながり、丸くふくらんで上で合わさる。
    /// 左右の端は、曲線の折り線でフラップを内側へ折り込んで閉じる。
    ///
    /// `len` は横の長さ、`height` は 1 枚の面の高さ（丸みに沿って測る。mm）。
    /// `bulge` はふくらみで、1 枚の面のふくらんだ部分が、下から上までに向きを変える角度（ラジアン）。
    /// `handle` があれば、上で合わさった縁を立てて貼り、その先を持ち手にする（持ち手の幅と高さ）。
    /// なければ、背面の上の縁ののりしろを正面の裏に貼る。
    /// `neck` が真なら、上の口の近くで面が逆向きに反って、まっすぐ立ち上がる（鳩サブレーの箱のような、下が広く上がすぼまる形）。
    ///
    /// 曲線の折り線の形は、ふくらみから自動で決まる。面を丸めたときに、折り線が 45 度に傾いた平面に乗る形にしてあり、
    /// フラップはその平面で映した位置（奥行き方向へまっすぐ倒れ込んだ形）に来る。
    pub fn pillow_with(len: f32, height: f32, t: f32, bulge: f32, handle_size: Option<HandleSize>, neck: bool) -> Template {
        let handle = handle_size.is_some();
        const MAIN: (f32, f32) = (0.0, 1.0);
        const SLICES: usize = 24;
        let psi = bulge.clamp(0.2, 1.6);
        let profile = PillowProfile::new(height, psi, neck);
        let depth = |v: f32| profile.depth(v);
        let deepest = (0..=64).map(|k| depth(height * k as f32 / 64.0)).fold(0.0f32, f32::max);
        let v_deepest = (0..=64).map(|k| height * k as f32 / 64.0).max_by(|a, b| depth(*a).total_cmp(&depth(*b))).unwrap_or(height * 0.5);
        let kappa = profile.kappa;
        let neck_bend = profile.neck;
        // 下の折り線での、1 枚の面の立ち上がりの傾き（2 枚が合わさる平面から外へ）と、上の端での傾き。
        let (start_tilt, end_tilt) = (profile.start, profile.end());
        // フラップの長さ（折り線から先）。閉じると、フラップは奥行きの向きへまっすぐ倒れ込むので、
        // 深さの 2 倍で向かいの面の縁に届く。紙の半分ほど手前で止めて、向かいの面を突き抜けないようにする。
        let reach = |v: f32| (2.0 * depth(v) - 0.5 * t).max(0.0);
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
            outline.extend((0..=SLICES).rev().map(|k| [crease(v_at(k)) + side * reach(v_at(k)), sign * v_at(k)]));
            (ccw(outline), [[crease(0.0), 0.0], [crease(v_deepest), sign * v_deepest], [crease(height), sign * height]], line)
        };

        let mut panels = vec![Panel {
            name: "正面",
            outline: face(1.0, 0.0),
            hinge: None,
            bend: Some(Bend { curvature: kappa, window: MAIN, line: Some(([0.0, 0.0], [len, 0.0])), neck: neck_bend }),
            curved: None,
            slices: SLICES,
            role: Role::Face,
                opening: 0,
        }];
        panels.push(Panel {
            name: "背面",
            outline: face(-1.0, under),
            hinge: Some(Hinge { parent: 0, a: [0.0, 0.0], b: [len, 0.0], angle: std::f32::consts::PI - 2.0 * start_tilt, window: MAIN, motion: Motion::Fixed }),
            bend: Some(Bend { curvature: kappa, window: MAIN, line: None, neck: neck_bend }),
            curved: None,
            slices: SLICES,
            role: Role::Face,
                opening: 0,
        });

        // 貼りしろと持ち手。上で合わさった 2 枚の縁を、端から端まで山折りでまっすぐ上に立てて、背中合わせに貼る。
        // これで上の口が閉じて、筒になる。持ち手は、貼りしろの中央から上へ続く。
        let fin = (0.08 * height).clamp(6.0, 10.0);
        let [handle_bottom, handle_top, handle_height] = handle_size.unwrap_or_default().resolve(len, height);
        let (h0, h1) = (0.5 * (len - handle_bottom), 0.5 * (len + handle_bottom));
        // 上の辺は、下の辺の中央に合わせる（下より広ければ、逆さの台形）。
        let inset = 0.5 * (handle_bottom - handle_top);
        if handle {
            for (fin_name, name, parent, sign, shift) in [("正面の貼りしろ", "正面の持ち手", 0usize, 1.0f32, 0.0f32), ("背面の貼りしろ", "背面の持ち手", 1usize, -1.0f32, under)] {
                let edge = sign * height;
                let base = edge + sign * fin;
                let fin_index = panels.len();
                panels.push(Panel::flat(
                    fin_name,
                    ccw(vec![[shift, edge], [len - shift, edge], [len - shift, base], [shift, base]]),
                    Some(Hinge { parent, a: [shift, edge], b: [len - shift, edge], angle: end_tilt, window: MAIN, motion: Motion::Fixed }),
                    Role::Fin,
                ));
                panels.push(Panel::flat(
                    name,
                    ccw(vec![[h0, base], [h1, base], [h1 - inset, base + sign * handle_height], [h0 + inset, base + sign * handle_height]]),
                    Some(Hinge { parent: fin_index, a: [h0, base], b: [h1, base], angle: 0.0, window: MAIN, motion: Motion::Fixed }),
                    Role::Face,
                ));
            }
        } else {
            // 背面の上の縁から、のりしろを内側へ折り返して、正面の裏に貼る。両端は、フラップの折り線に掛からないよう斜めに落とす。
            let tab = (0.12 * height).clamp(8.0, 14.0);
            let (edge, base) = (-height, -height - tab);
            let slant = tab * 0.8;
            // 平らなのりしろが丸い正面を突き抜けて見えないよう、ほんの少しだけ内側へ倒しておく。
            const TUCKED_IN: f32 = 0.2;
            panels.push(Panel::flat(
                "のりしろ",
                ccw(vec![[under, edge], [len - under, edge], [len - under - slant, base], [under + slant, base]]),
                Some(Hinge { parent: 1, a: [under, edge], b: [len - under, edge], angle: std::f32::consts::PI + 2.0 * end_tilt + TUCKED_IN, window: MAIN, motion: Motion::Fixed }),
                // 内側へ折り返して正面の裏に貼るので、外からは見えない（柄も置かない）。
                // 背面の上の辺は、のりしろへの折り線ではなく、正面の上の辺との合わせ目として扱われる（柄が上の縁を越えて続く）。
                Role::GlueTab,
            ));
        }

        // 左右のフラップ。正面のものが外側に来る。左の口が開け口 0、右の口が開け口 1（ピロー型には上と底の区別がない）。
        // 開けるときは、外側の正面のフラップが先に起き、そのあとで内側の背面のフラップが起きる。
        for (name, parent, sign, shift, hidden, opens) in [("正面のフラップ", 0usize, 1.0f32, 0.0f32, false, (0.0, 0.6)), ("背面のフラップ", 1usize, -1.0f32, under, true, (0.4, 1.0))] {
            for (side, opening) in [(-1.0f32, 0), (1.0, 1)] {
                let (outline, crease, line) = flap(sign, shift, side);
                panels.push(Panel { name, outline, hinge: None, bend: None, curved: Some(CurvedFold { parent, crease, line, opens: Some(opens) }), slices: SLICES, role: if hidden { Role::Flap } else { Role::Face }, opening });
            }
        }

        let (min, max) = bounds(&panels);
        let r = len.min(height) * 0.26;
        let marks = vec![([len * 0.5, height * 0.5], r), ([deepest, v_deepest], r * 0.55)];
        // 手を入れる穴。持ち手の高さの中ほど（台形のその高さでの幅）と、高さに合わせる。
        let hole = |sign: f32| [len * 0.5, sign * (height + fin + handle_height * 0.5), 0.33 * 0.5 * (handle_bottom + handle_top), (handle_height * 0.22).min(12.0)];
        let holes = if handle { vec![hole(1.0), hole(-1.0)] } else { Vec::new() };
        // 端のフラップは、紙の張りで閉じた状態を保つ。
        let openings = openings(&panels, &[("正面のフラップ", None, Hold::Tension)]);
        let glued = if handle { glued(&panels, &[("正面の貼りしろ", "背面の貼りしろ"), ("正面の持ち手", "背面の持ち手")]) } else { glued(&panels, &[("のりしろ", "正面")]) };
        // 根の面は下の折り線から巻きはじめるので、そのままだと全体が後ろへ傾く。立ち上がりの傾きぶん手前へ起こして、まっすぐ立てる。
        Template {
            panels,
            thickness: t,
            min,
            max,
            world: upright_world([len * 0.5, height * 0.5], deepest * 2.0, t),
            tilt: start_tilt,
            tilt_window: MAIN,
            marks,
            holes,
            open_angle: 0.0,
            openings,
            glued,
            drawn: None,
        }
    }
}

/// ピロー型の 1 枚の面の、横から見た形。下の折り線から上の端まで、丸みに沿った距離 v で表す。
/// 傾きは、2 枚が合わさる平面から外へ向かう角度。曲がり具合が正なら、上へ行くほど内側へ向く。
pub(super) struct PillowProfile {
    height: f32,
    /// 下の折り線での傾き。
    pub(super) start: f32,
    /// ふくらんだ部分の曲がり具合。
    pub(super) kappa: f32,
    /// 首（上の口の近くで逆に反る部分）のはじまりと、その曲がり具合。
    pub(super) neck: Option<(f32, f32)>,
}

/// 首のはじまる高さ（面の高さに対する割合）。
const NECK_AT: f32 = 0.72;

impl PillowProfile {
    pub(super) fn new(height: f32, psi: f32, neck: bool) -> PillowProfile {
        if !neck {
            // 上下が対称。下で外へ psi/2 傾いて立ち上がり、上で内へ psi/2 傾いて合わさる。
            return PillowProfile { height, start: psi * 0.5, kappa: psi / height, neck: None };
        }
        // ふくらんだ部分で psi だけ向きを変え、首で逆に反って、上の端でまっすぐ（傾き 0）になる。
        // そのうえで上の端がちょうど合わさる（深さ 0 になる）ように、立ち上がりの傾きを二分法で決める。
        let at = height * NECK_AT;
        let kappa = psi / at;
        let make = |start: f32| {
            let turned = start - psi;
            PillowProfile { height, start, kappa, neck: Some((at, turned / (height - at))) }
        };
        let (mut lo, mut hi) = (0.0f32, psi);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if make(mid).depth(height) > 0.0 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        make(0.5 * (lo + hi))
    }

    /// 一定の曲がり具合 `k` の区間を、傾き `tilt` から長さ `u` だけ進んだときの、外への張り出し。
    fn stretch(tilt: f32, k: f32, u: f32) -> f32 {
        if k.abs() < 1e-7 {
            u * tilt.sin()
        } else {
            ((tilt - k * u).cos() - tilt.cos()) / k
        }
    }

    /// 高さ v での、ふくらみの深さ（2 枚が合わさる平面からの距離）。
    pub(super) fn depth(&self, v: f32) -> f32 {
        let v = v.clamp(0.0, self.height);
        match self.neck {
            Some((at, k)) if v > at => Self::stretch(self.start, self.kappa, at) + Self::stretch(self.start - self.kappa * at, k, v - at),
            _ => Self::stretch(self.start, self.kappa, v),
        }
    }

    /// 上の端での傾き。
    pub(super) fn end(&self) -> f32 {
        match self.neck {
            Some((at, k)) => self.start - self.kappa * at - k * (self.height - at),
            None => self.start - self.kappa * self.height,
        }
    }
}
