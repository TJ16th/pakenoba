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
                    // お客さんが開けるのは、上だけ。フタを起こしたあとで、左右のフラップを外へ起こす。
                    Some(Hinge { parent, a: [u0, side_edge], b: [u1, side_edge], angle: quarter, window: dust_window, motion: if sign > 0.0 { Motion::Flap(FLAP_LIFT) } else { Motion::Fixed } }),
                    Role::Flap,
                ));
            }

            // フタ。背面につながり、奥行きより紙 1 枚ぶん短い。
            let lid_far = lid_edge + sign * (d - t);
            let lid_index = panels.len();
            // お客さんが開けるのは、上のフタ。差し込みは、正面の内側に沿って抜ける。
            let clear = (tuck / (d - t)).min(1.0).asin();
            let (lid_motion, tuck_motion) = if sign > 0.0 { (Motion::Lift(LID_LIFT), Motion::Slide { lift: LID_LIFT, clear }) } else { (Motion::Fixed, Motion::Fixed) };
            panels.push(Panel::flat(names[2], rect(u_b, lid_edge, u_b + w, lid_far), Some(Hinge { parent: 3, a: [u_b, lid_edge], b: [u_b + w, lid_edge], angle: quarter, window: lid_window, motion: lid_motion }), Role::Face));

            // 差し込み（ベロ）。フタの先につながり、正面の内側に入る。
            let (s0, s1) = (u_b + 0.5, u_b + w - 0.5);
            panels.push(Panel::flat(
                names[3],
                ccw(vec![[s0, lid_far], [s1, lid_far], [s1 - tuck_taper, lid_far + sign * tuck], [s0 + tuck_taper, lid_far + sign * tuck]]),
                Some(Hinge { parent: lid_index, a: [s0, lid_far], b: [s1, lid_far], angle: quarter, window: tuck_window, motion: tuck_motion }),
                Role::Tuck,
            ));
        }

        let (min, max) = bounds(&panels);
        let r = w.min(d).min(h) * 0.34;
        let marks = vec![([u_r, h * 0.55], r), ([u_b + w * 0.5, h + t], r * 0.8)];
        let openings = openings(&panels, &[("上のフタ", Some("上の差し込み"), Hold::Nothing), ("底のフタ", Some("底の差し込み"), Hold::Nothing)]);
        let glued = glued(&panels, &[("のりしろ", "背面")]);
        Template { panels, thickness: t, min, max, world: upright_world([u_f + w * 0.5, h * 0.5], d, t), tilt: 0.0, tilt_window: (0.0, 0.5), marks, holes: Vec::new(), open_angle: 0.0, openings, glued }
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
            for wall in (1..n).filter(|wall| !tuck_walls.contains(wall)) {
                let a = wall_start(wall) + 0.5;
                let b = (if wall == n - 1 { last_end } else { wall_start(wall + 1) }) - 0.5;
                panels.push(Panel::flat(
                    flap_name,
                    ccw(vec![[a, side_edge], [b, side_edge], [b - inset, side_edge + sign * flap], [a + inset, side_edge + sign * flap]]),
                    Some(Hinge { parent: wall, a: [a, side_edge], b: [b, side_edge], angle: quarter, window: flap_window, motion: if sign > 0.0 { Motion::Flap(FLAP_LIFT) } else { Motion::Fixed } }),
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
            // お客さんが開けるのは、上のフタ。
            let top = sign > 0.0;
            panels.push(Panel::flat(lid_name, ccw(lid.clone()), Some(Hinge { parent: 0, a: [u0, lid_edge], b: [u0 + side, lid_edge], angle: quarter, window: lid_window, motion: if top { Motion::Lift(LID_LIFT) } else { Motion::Fixed } }), Role::Face));
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
                let motion = if top { Motion::Slide { lift: LID_LIFT, clear: (tuck / nearest).min(1.0).asin() } } else { Motion::Fixed };
                panels.push(Panel::flat(tuck_name, ccw(vec![a, b, tip(b, -taper), tip(a, taper)]), Some(Hinge { parent: lid_index, a, b, angle: quarter, window: tuck_window, motion }), Role::Tuck));
            }
        }

        let (min, max) = bounds(&panels);
        let r = side.min(h) * 0.3;
        let marks = vec![([u0 + side, h * 0.55], r), ([u0 + side * 0.5, h + t], r * 0.8)];
        let depth = side / (std::f32::consts::PI / n as f32).tan();
        let openings = openings(&panels, &[("上のフタ", Some("上の差し込み"), Hold::Nothing), ("底のフタ", Some("底の差し込み"), Hold::Nothing)]);
        let glued = glued(&panels, &[("のりしろ", "側面 1")]);
        Template { panels, thickness: t, min, max, world: upright_world([u0 + side * 0.5, h * 0.5], depth, t), tilt: 0.0, tilt_window: (0.0, 0.5), marks, holes: Vec::new(), open_angle: 0.0, openings, glued }
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

        let mut panels = vec![Panel::flat("底", rect(0.0, 0.0, w, d), None, Role::Face)];
        panels.push(Panel { name: "正面", outline: rect(0.0, d, w, top), hinge: Some(Hinge { parent: 0, a: [0.0, d], b: [w, d], angle: rise, window: MAIN, motion: Motion::Fixed }), bend: bend(), curved: None, slices: 0, role: Role::Face });
        panels.push(Panel { name: "背面", outline: rect(0.0, -l, w, 0.0), hinge: Some(Hinge { parent: 0, a: [0.0, 0.0], b: [w, 0.0], angle: rise, window: MAIN, motion: Motion::Fixed }), bend: bend(), curved: None, slices: 0, role: Role::Face });
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
        Template { panels, thickness: t, min, max, world, tilt: std::f32::consts::FRAC_PI_2, tilt_window: (0.0, 0.5), marks, holes: vec![hole(top, 1.0), hole(-l, -1.0)], open_angle: 0.0, openings: Vec::new(), glued: Vec::new() }
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

        // 底は差し込み箱と同じ。
        let clear = t + 0.5;
        for (name, parent, u0, u1) in [("底の左フラップ", 1usize, u_l, u_f - clear), ("底の右フラップ", 2usize, u_r + clear, u_b - 0.5)] {
            let taper = (u1 - u0) * 0.15;
            panels.push(Panel::flat(
                name,
                ccw(vec![[u0, 0.0], [u1, 0.0], [u1 - taper, -dust], [u0 + taper, -dust]]),
                Some(Hinge { parent, a: [u0, 0.0], b: [u1, 0.0], angle: quarter, window: (0.50, 0.70), motion: fixed }),
                Role::Flap,
            ));
        }
        let lid_far = -t - (d - t);
        let bottom_lid = panels.len();
        panels.push(Panel::flat("底のフタ", rect(u_b, -t, u_b + w, lid_far), Some(Hinge { parent: 3, a: [u_b, -t], b: [u_b + w, -t], angle: quarter, window: (0.75, 1.0), motion: fixed }), Role::Face));
        let (s0, s1) = (u_b + 0.5, u_b + w - 0.5);
        panels.push(Panel::flat(
            "底の差し込み",
            ccw(vec![[s0, lid_far], [s1, lid_far], [s1 - tuck_taper, lid_far - tuck], [s0 + tuck_taper, lid_far - tuck]]),
            Some(Hinge { parent: bottom_lid, a: [s0, lid_far], b: [s1, lid_far], angle: quarter, window: (0.60, 0.80), motion: fixed }),
            Role::Tuck,
        ));

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
            role: Role::Face,
        }];
        panels.push(Panel {
            name: "背面",
            outline: face(-1.0, under),
            hinge: Some(Hinge { parent: 0, a: [0.0, 0.0], b: [len, 0.0], angle: std::f32::consts::PI - psi, window: MAIN, motion: Motion::Fixed }),
            bend: Some(Bend { curvature: kappa, window: MAIN, line: None }),
            curved: None,
            slices: SLICES,
            role: Role::Face,
        });

        // 貼りしろと持ち手。上で合わさった 2 枚の縁を、端から端まで山折りでまっすぐ上に立てて、背中合わせに貼る。
        // これで上の口が閉じて、筒になる。持ち手は、貼りしろの中央から上へ続く。
        let fin = (0.08 * height).clamp(6.0, 10.0);
        let handle_height = (0.35 * height).clamp(25.0, 50.0);
        let (h0, h1) = (len * 0.27, len * 0.73);
        let inset = (h1 - h0) * 0.09;
        for (fin_name, name, parent, sign, shift) in [("正面の貼りしろ", "正面の持ち手", 0usize, 1.0f32, 0.0f32), ("背面の貼りしろ", "背面の持ち手", 1usize, -1.0f32, under)] {
            let edge = sign * height;
            let base = edge + sign * fin;
            let fin_index = panels.len();
            panels.push(Panel::flat(
                fin_name,
                ccw(vec![[shift, edge], [len - shift, edge], [len - shift, base], [shift, base]]),
                Some(Hinge { parent, a: [shift, edge], b: [len - shift, edge], angle: -half_turn, window: MAIN, motion: Motion::Fixed }),
                Role::Fin,
            ));
            panels.push(Panel::flat(
                name,
                ccw(vec![[h0, base], [h1, base], [h1 - inset, base + sign * handle_height], [h0 + inset, base + sign * handle_height]]),
                Some(Hinge { parent: fin_index, a: [h0, base], b: [h1, base], angle: 0.0, window: MAIN, motion: Motion::Fixed }),
                Role::Face,
            ));
        }

        // 左右のフラップ。正面のものが外側に来る。
        for (name, parent, sign, shift, hidden) in [("正面のフラップ", 0usize, 1.0f32, 0.0f32, false), ("背面のフラップ", 1usize, -1.0f32, under, true)] {
            for side in [-1.0f32, 1.0] {
                let (outline, crease, line) = flap(sign, shift, side);
                panels.push(Panel { name, outline, hinge: None, bend: None, curved: Some(CurvedFold { parent, crease, line }), slices: SLICES, role: if hidden { Role::Flap } else { Role::Face } });
            }
        }

        let (min, max) = bounds(&panels);
        let r = len.min(height) * 0.26;
        let marks = vec![([len * 0.5, height * 0.5], r), ([deepest, height * 0.5], r * 0.55)];
        let hole = |sign: f32| [len * 0.5, sign * (height + fin + handle_height * 0.5), (h1 - h0) * 0.3, handle_height * 0.22];
        // 端のフラップは、紙の張りで閉じた状態を保つ。
        let openings = openings(&panels, &[("正面のフラップ", None, Hold::Tension)]);
        let glued = glued(&panels, &[("正面の貼りしろ", "背面の貼りしろ"), ("正面の持ち手", "背面の持ち手")]);
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
            openings,
            glued,
        }
    }

}
