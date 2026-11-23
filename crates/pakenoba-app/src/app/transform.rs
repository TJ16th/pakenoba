//! 選んだ柄の、大きさと回転のつまみ（Photoshop の自由変形と同じ形）。展開図でも、立体の上でも使う。
//!
//! - 四隅のつまみをドラッグすると、大きさが変わる。はじめは縦横比を保つ。Shift を押しているあいだは比率を外す。
//!   Alt を押しているあいだは、中心を動かさない（ふだんは、向かいの角が動かない）。
//! - 角の少し外側をドラッグすると、回る。Shift を押しているあいだは 15 度ずつ。
//! - プロパティの鎖のボタンを外していると、比率はふだんから外れ、Shift で保つ（Photoshop と同じ）。
//!
//! 計算は、どれも展開図の座標（mm）で行う。画面との行き来は、それぞれの表示が受け持つ。

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, pos2, vec2};

use pakenoba_core::math::{Affine, V3, cross, dot, sub};
use pakenoba_core::surface::Decal;
use pakenoba_core::template::{P2, Template};

/// 角の並び（柄の座標での符号）。左下、右下、右上、左上。
const CORNERS: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
/// 角のつまみの大きさと、回す場所の広さ（画面の点）。
const KNOB: f32 = 8.0;
const TURN_REACH: f32 = 22.0;
/// 柄の大きさの下限（mm）。
const MIN_MM: f32 = 1.0;

/// つかんだつまみ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    /// 角（`CORNERS` の番号）で、大きさを変える。
    Corner(usize),
    /// 角の外側で、回す。
    Turn,
}

/// 変形の途中。はじめの柄と、はじめに押した点（展開図の座標）を覚えておく。
#[derive(Clone, Copy, Debug)]
pub struct Transforming {
    pub index: usize,
    pub handle: Handle,
    pub start: Decal,
    pub pressed: P2,
}

/// 柄の四隅（展開図の座標）。
pub fn corners(decal: &Decal, tpl: &Template) -> [P2; 4] {
    let place = decal.place(tpl);
    CORNERS.map(|s| place.point([s[0] * decal.size[0] * 0.5, s[1] * decal.size[1] * 0.5]))
}

/// 画面の点 `p` が、どのつまみの上か。`screen` は画面での四隅。
pub fn handle_at(screen: [Pos2; 4], p: Pos2) -> Option<Handle> {
    if let Some(i) = (0..4).find(|&i| (screen[i] - p).length() <= KNOB) {
        return Some(Handle::Corner(i));
    }
    let near_corner = screen.iter().any(|c| (*c - p).length() <= TURN_REACH);
    (near_corner && !inside(screen, p)).then_some(Handle::Turn)
}

fn inside(quad: [Pos2; 4], p: Pos2) -> bool {
    let side = |a: Pos2, b: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let s: Vec<f32> = (0..4).map(|i| side(quad[i], quad[(i + 1) % 4])).collect();
    s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
}

/// つまみを `at`（展開図の座標）まで動かしたときの柄。`free` は縦横比を外す、`from_center` は中心を動かさない、
/// `snap` は 15 度ずつ回す。
pub fn apply(t: &Transforming, tpl: &Template, at: P2, free: bool, from_center: bool, snap: bool) -> Decal {
    let mut decal = t.start;
    let place = t.start.place(tpl);
    match t.handle {
        Handle::Corner(i) => {
            let q = place.inverse().point(at);
            let half = [t.start.size[0] * 0.5, t.start.size[1] * 0.5];
            let s = CORNERS[i];
            let corner = [s[0] * half[0], s[1] * half[1]];
            let anchor = if from_center { [0.0, 0.0] } else { [-corner[0], -corner[1]] };
            let reach = [corner[0] - anchor[0], corner[1] - anchor[1]];
            // 新しい角の場所（柄の座標）。比率を保つときは、もとの対角線の上に寄せる。
            let new_corner = if free {
                // 向かいの角を越えないようにする。
                let min = |r: f32, sign: f32| sign * (sign * r).max(MIN_MM * if from_center { 0.5 } else { 1.0 });
                [anchor[0] + min(q[0] - anchor[0], s[0]), anchor[1] + min(q[1] - anchor[1], s[1])]
            } else {
                let length = reach[0] * reach[0] + reach[1] * reach[1];
                let f = (((q[0] - anchor[0]) * reach[0] + (q[1] - anchor[1]) * reach[1]) / length.max(1e-6)).max(MIN_MM / t.start.size[0].min(t.start.size[1]).max(MIN_MM));
                [anchor[0] + reach[0] * f, anchor[1] + reach[1] * f]
            };
            let (size, middle) = if from_center {
                ([2.0 * new_corner[0].abs(), 2.0 * new_corner[1].abs()], [0.0, 0.0])
            } else {
                ([(new_corner[0] - anchor[0]).abs(), (new_corner[1] - anchor[1]).abs()], [(new_corner[0] + anchor[0]) * 0.5, (new_corner[1] + anchor[1]) * 0.5])
            };
            decal.size = [size[0].max(MIN_MM), size[1].max(MIN_MM)];
            decal.set_center(tpl, place.point(middle));
        }
        Handle::Turn => {
            let c = t.start.center(tpl);
            let a0 = (t.pressed[1] - c[1]).atan2(t.pressed[0] - c[0]);
            let a1 = (at[1] - c[1]).atan2(at[0] - c[0]);
            let mut angle = t.start.angle + (a1 - a0);
            if snap {
                let step = 15f32.to_radians();
                angle = (angle / step).round() * step;
            }
            decal.angle = (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        }
    }
    decal
}

/// 画面に、枠と四隅のつまみを描く。
pub fn paint(painter: &egui::Painter, screen: [Pos2; 4], color: Color32) {
    for i in 0..4 {
        painter.line_segment([screen[i], screen[(i + 1) % 4]], Stroke::new(1.0, color));
    }
    for c in screen {
        let knob = Rect::from_center_size(c, vec2(KNOB, KNOB));
        painter.rect_filled(knob, 1.0, Color32::WHITE);
        painter.rect_stroke(knob, 1.0, Stroke::new(1.2, color), egui::StrokeKind::Middle);
    }
}

/// つまみの上でのカーソル。角は、画面での向きに合わせた斜めの矢印。
pub fn cursor(screen: [Pos2; 4], handle: Handle) -> egui::CursorIcon {
    match handle {
        Handle::Corner(i) => {
            let center = pos2(screen.iter().map(|p| p.x).sum::<f32>() / 4.0, screen.iter().map(|p| p.y).sum::<f32>() / 4.0);
            let d = screen[i] - center;
            if (d.x >= 0.0) == (d.y >= 0.0) { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw }
        }
        Handle::Turn => egui::CursorIcon::Alias,
    }
}

/// 立体の上で、画面の光線（`origin` から `dir`）が、柄の置かれた面を広げた平面と交わる点（展開図の座標）。
/// `frame` は、その面の展開図の座標から立体への写し。
pub fn ray_to_sheet(frame: &Affine, origin: V3, dir: V3) -> Option<P2> {
    let col = |c: usize| [frame.m[0][c], frame.m[1][c], frame.m[2][c]];
    let (a, b) = (col(0), col(1));
    // u a + v b - t dir = origin - frame.t を解く。
    let rhs = sub(origin, frame.t);
    let neg = [-dir[0], -dir[1], -dir[2]];
    let det = dot(a, cross(b, neg));
    if det.abs() < 1e-9 {
        return None;
    }
    let u = dot(rhs, cross(b, neg)) / det;
    let v = dot(a, cross(rhs, neg)) / det;
    let t = dot(a, cross(b, rhs)) / det;
    (t > 0.0).then_some([u, v])
}

impl super::App {
    /// つまみを出す柄。選んでいる柄が 1 つで、`back` の側にあり、隠しても、ロックしてもいないとき。
    /// 手のひらの道具では出さない。`back` が `None` なら、側を問わない（立体の表示）。
    pub(super) fn knob_target(&self, back: impl Into<Option<bool>>) -> Option<usize> {
        let back = back.into();
        if self.tool == super::Tool::Hand || !self.extra.is_empty() {
            return None;
        }
        let index = self.selected?;
        let placed = self.decals.get(index)?;
        (back.is_none_or(|b| b == placed.back) && !placed.layer.locked && !placed.layer.hidden).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pakenoba_core::math::Affine;

    fn tuck() -> Template {
        Template::tuck_box(80.0, 50.0, 120.0, 0.5)
    }

    fn sample(tpl: &Template) -> Decal {
        let panel = tpl.panels.iter().position(|p| p.name == "正面").unwrap();
        Decal { panel, at: [0.5, 0.5], size: [40.0, 20.0], angle: 0.0 }
    }

    #[test]
    fn 角を引くと_比率を保って大きくなり_向かいの角は動かない() {
        let tpl = tuck();
        let start = sample(&tpl);
        let before = corners(&start, &tpl);
        let t = Transforming { index: 0, handle: Handle::Corner(2), start, pressed: before[2] };
        let out = apply(&t, &tpl, [before[2][0] + 40.0, before[2][1] + 5.0], false, false, false);
        assert!((out.size[0] / out.size[1] - 2.0).abs() < 1e-4, "{:?}", out.size);
        assert!(out.size[0] > 40.0);
        let after = corners(&out, &tpl);
        assert!((after[0][0] - before[0][0]).abs() < 1e-3 && (after[0][1] - before[0][1]).abs() < 1e-3);
    }

    #[test]
    fn shift_で比率を外し_alt_で中心を保つ() {
        let tpl = tuck();
        let start = sample(&tpl);
        let before = corners(&start, &tpl);
        let t = Transforming { index: 0, handle: Handle::Corner(2), start, pressed: before[2] };
        let free = apply(&t, &tpl, [before[2][0] + 20.0, before[2][1]], true, false, true);
        assert!((free.size[0] - 60.0).abs() < 1e-3 && (free.size[1] - 20.0).abs() < 1e-3, "{:?}", free.size);
        let centered = apply(&t, &tpl, [before[2][0] + 10.0, before[2][1] + 5.0], true, true, true);
        let (c0, c1) = (start.center(&tpl), centered.center(&tpl));
        assert!((c0[0] - c1[0]).abs() < 1e-3 && (c0[1] - c1[1]).abs() < 1e-3);
        assert!((centered.size[0] - 60.0).abs() < 1e-3 && (centered.size[1] - 30.0).abs() < 1e-3, "{:?}", centered.size);
    }

    #[test]
    fn 向かいの角を越えても_裏返らずに小さく止まる() {
        let tpl = tuck();
        let start = sample(&tpl);
        let before = corners(&start, &tpl);
        let t = Transforming { index: 0, handle: Handle::Corner(2), start, pressed: before[2] };
        for free in [false, true] {
            let out = apply(&t, &tpl, [before[0][0] - 30.0, before[0][1] - 30.0], free, false, free);
            assert!(out.size[0] >= MIN_MM && out.size[1] >= MIN_MM && out.size[0] < 5.0, "{:?}", out.size);
        }
    }

    #[test]
    fn 外側で回し_shift_で15度ずつ() {
        let tpl = tuck();
        let start = sample(&tpl);
        let c = start.center(&tpl);
        let t = Transforming { index: 0, handle: Handle::Turn, start, pressed: [c[0] + 30.0, c[1]] };
        let quarter = apply(&t, &tpl, [c[0], c[1] + 30.0], false, false, false);
        assert!((quarter.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        let snapped = apply(&t, &tpl, [c[0] + 30.0, c[1] + 9.0], true, false, true);
        assert!((snapped.angle - 15f32.to_radians()).abs() < 1e-4, "{}", snapped.angle.to_degrees());
    }

    #[test]
    fn つまみの当たり() {
        let quad = [pos2(0.0, 100.0), pos2(100.0, 100.0), pos2(100.0, 0.0), pos2(0.0, 0.0)];
        assert_eq!(handle_at(quad, pos2(103.0, 3.0)), Some(Handle::Corner(2)));
        assert_eq!(handle_at(quad, pos2(114.0, -10.0)), Some(Handle::Turn));
        assert_eq!(handle_at(quad, pos2(50.0, 50.0)), None);
        assert_eq!(handle_at(quad, pos2(90.0, 12.0)), None, "内側は、つかんで動かす場所");
    }

    #[test]
    fn 光線と面の交わり() {
        let frame = Affine::IDENTITY;
        let p = ray_to_sheet(&frame, [10.0, 20.0, 50.0], [0.0, 0.0, -1.0]).unwrap();
        assert!((p[0] - 10.0).abs() < 1e-4 && (p[1] - 20.0).abs() < 1e-4);
        assert!(ray_to_sheet(&frame, [0.0, 0.0, 50.0], [0.0, 0.0, 1.0]).is_none());
    }
}
