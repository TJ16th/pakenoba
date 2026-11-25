//! 道具のアイコン。線だけの単色で、その場で描く（アイコンのフォントや画像を持たない）。

use pakenoba_i18n::t;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};

use super::Tool;

/// 道具の帯のボタン。アイコンだけなので、必ずツールチップを付ける。
pub(super) fn tool_button(ui: &mut egui::Ui, tool: Tool, selected: bool) -> egui::Response {
    strip_button(ui, tool.ready(), selected, tool.tip(), |painter, center, stroke| draw(painter, center, tool, stroke))
}

/// 道具の帯の下の、素材の欄を開け閉めするボタン。
pub(super) fn assets_button(ui: &mut egui::Ui, open: bool) -> egui::Response {
    strip_button(ui, true, open, t!("Open or close the assets panel"), |painter, center, stroke| {
        // 重ねた 2 枚の紙。
        let p = |x: f32, y: f32| pos2(center.x + x - 10.0, center.y + y - 10.0);
        painter.add(Shape::line(vec![p(6.0, 6.0), p(6.0, 3.5), p(16.5, 3.5), p(16.5, 13.0), p(14.0, 13.0)], stroke));
        painter.rect_stroke(Rect::from_min_max(p(3.5, 6.5), p(14.0, 16.5)), 1.0, stroke, StrokeKind::Middle);
    })
}

fn strip_button(ui: &mut egui::Ui, ready: bool, selected: bool, tip: &str, icon: impl FnOnce(&egui::Painter, Pos2, Stroke)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(34.0, 34.0), if ready { Sense::click() } else { Sense::hover() });
    let visuals = ui.visuals();
    let (fill, color) = if selected {
        (visuals.selection.bg_fill, Color32::WHITE)
    } else if !ready {
        (Color32::TRANSPARENT, visuals.weak_text_color().gamma_multiply(0.6))
    } else if response.hovered() {
        (visuals.widgets.hovered.bg_fill, visuals.strong_text_color())
    } else {
        (Color32::TRANSPARENT, visuals.text_color())
    };
    let painter = ui.painter();
    painter.rect_filled(rect.shrink(1.0), 4.0, fill);
    icon(painter, rect.center(), Stroke::new(1.4, color));
    response.on_hover_text(tip)
}

/// 20 × 20 の枠の中に描く。座標は、枠の左上から。
fn draw(painter: &egui::Painter, center: Pos2, tool: Tool, stroke: Stroke) {
    let p = |x: f32, y: f32| pos2(center.x + x - 10.0, center.y + y - 10.0);
    let path = |points: &[(f32, f32)]| points.iter().map(|&(x, y)| p(x, y)).collect::<Vec<Pos2>>();
    match tool {
        Tool::Select => {
            // 矢印のカーソル。
            painter.add(Shape::closed_line(path(&[(5.0, 3.0), (5.0, 16.0), (8.5, 12.8), (11.0, 18.0), (13.0, 17.1), (10.6, 12.0), (15.0, 12.0)]), stroke));
        }
        Tool::Text => {
            // T の字。
            for (a, b) in [((5.0, 5.0), (15.0, 5.0)), ((10.0, 5.0), (10.0, 16.0)), ((5.0, 5.0), (5.0, 7.2)), ((15.0, 5.0), (15.0, 7.2)), ((8.0, 16.0), (12.0, 16.0))] {
                painter.line_segment([p(a.0, a.1), p(b.0, b.1)], stroke);
            }
        }
        Tool::Shape => {
            // 四角と丸。
            painter.rect_stroke(Rect::from_min_max(p(3.0, 7.5), p(12.0, 16.5)), 1.0, stroke, StrokeKind::Middle);
            painter.circle_stroke(p(13.0, 8.0), 4.5, stroke);
        }
        Tool::Ai => {
            // きらめき。
            painter.add(Shape::closed_line(path(&[(10.0, 2.0), (11.6, 8.4), (18.0, 10.0), (11.6, 11.6), (10.0, 18.0), (8.4, 11.6), (2.0, 10.0), (8.4, 8.4)]), stroke));
        }
        Tool::Hand => {
            // 開いた手。4 本の指と、親指から手のひらへの輪郭。
            for (x, top, bottom) in [(7.2, 5.0, 10.0), (9.6, 3.4, 10.0), (12.0, 4.0, 10.0), (14.4, 6.0, 10.5)] {
                painter.line_segment([p(x, top), p(x, bottom)], stroke);
            }
            painter.add(Shape::line(path(&[(7.2, 10.0), (7.2, 12.6), (5.2, 10.6), (3.8, 11.8), (7.0, 16.4), (8.6, 17.6), (12.8, 17.6), (14.4, 15.2), (14.4, 10.5)]), stroke));
        }
    }
}

/// レイヤーの行の、小さなアイコンのボタン（目と鍵）。`on` のときは、はっきり描く。
fn row_button(ui: &mut egui::Ui, on: bool, tip: &str, icon: impl FnOnce(&egui::Painter, Pos2, Stroke)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(20.0, 18.0), Sense::click());
    let visuals = ui.visuals();
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, visuals.widgets.hovered.bg_fill);
    }
    let color = if on { visuals.text_color() } else { visuals.weak_text_color().gamma_multiply(0.7) };
    icon(ui.painter(), rect.center(), Stroke::new(1.2, color));
    response.on_hover_text(tip)
}

/// 柄を見せるか隠すかの、目のボタン。
pub(super) fn eye_button(ui: &mut egui::Ui, visible: bool) -> egui::Response {
    let tip = if visible { t!("Hide (not drawn on the box or the dieline)") } else { t!("Show") };
    row_button(ui, visible, tip, |painter, c, stroke| {
        // 目の輪郭（上下 2 本の弧）と、ひとみ。隠しているときは、閉じた目（下の弧だけ）。
        let arc = |sign: f32| -> Vec<Pos2> { (0..=12).map(|k| { let t = k as f32 / 12.0; let x = (t - 0.5) * 13.0; pos2(c.x + x, c.y + sign * 4.2 * (1.0 - (2.0 * t - 1.0).powi(2))) }).collect() };
        painter.add(Shape::line(arc(1.0), stroke));
        if visible {
            painter.add(Shape::line(arc(-1.0), stroke));
            painter.circle_filled(c, 1.9, stroke.color);
        }
    })
}

/// 筆（画像をロック）。16 × 16 の枠の中。
pub(super) fn draw_brush(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    painter.line_segment([c + vec2(-1.0, 1.0), c + vec2(5.0, -5.0)], Stroke::new(stroke.width + 0.6, stroke.color));
    painter.add(Shape::convex_polygon(vec![c + vec2(-1.8, 0.2), c + vec2(0.2, 1.8), c + vec2(-2.5, 4.8), c + vec2(-5.2, 5.2), c + vec2(-4.8, 2.5)], stroke.color, Stroke::NONE));
}

/// 四方の矢印（位置をロック）。
pub(super) fn draw_move(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    painter.line_segment([c + vec2(-6.0, 0.0), c + vec2(6.0, 0.0)], stroke);
    painter.line_segment([c + vec2(0.0, -6.0), c + vec2(0.0, 6.0)], stroke);
    for (tip, a, b) in [((6.0, 0.0), (3.8, -2.2), (3.8, 2.2)), ((-6.0, 0.0), (-3.8, -2.2), (-3.8, 2.2)), ((0.0, 6.0), (-2.2, 3.8), (2.2, 3.8)), ((0.0, -6.0), (-2.2, -3.8), (2.2, -3.8))] {
        painter.add(Shape::line(vec![c + vec2(a.0, a.1), c + vec2(tip.0, tip.1), c + vec2(b.0, b.1)], stroke));
    }
}

/// 鍵（すべてをロック）。
pub(super) fn draw_lock(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    painter.rect_stroke(Rect::from_center_size(c + vec2(0.0, 2.2), vec2(9.0, 6.5)), 1.0, stroke, StrokeKind::Middle);
    let shackle: Vec<Pos2> = (0..=10).map(|k| { let a = std::f32::consts::PI * k as f32 / 10.0; pos2(c.x - 2.8 * a.cos(), c.y - 1.0 - 3.3 * a.sin()) }).collect();
    painter.add(Shape::line(shackle, stroke));
}

/// 四角に丸の穴（マスクを足す。Photoshop の下の段と同じ形）。
pub(super) fn draw_mask(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    painter.rect_stroke(Rect::from_center_size(c, vec2(14.0, 11.0)), 1.0, stroke, StrokeKind::Middle);
    painter.circle_filled(c, 3.3, stroke.color);
}

/// 半分ぬった丸（調整）。
pub(super) fn draw_adjust(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    painter.circle_stroke(c, 6.0, stroke);
    let half: Vec<Pos2> = (0..=16).map(|k| { let a = std::f32::consts::PI * (0.5 + k as f32 / 16.0); c + vec2(6.0 * a.cos(), -6.0 * a.sin()) }).collect();
    painter.add(Shape::convex_polygon(half, stroke.color, Stroke::NONE));
}

/// 加工（箔押し、エンボスなど）。浮き出した版の形。
pub(super) fn draw_foil(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    // 浮き出した版（外の四角、内の四角、角をつなぐ斜めの線）。生成AIの道具の「きらめき」と見分けられるように。
    let p = |x: f32, y: f32| c + vec2(x, y);
    painter.rect_stroke(Rect::from_min_max(p(-7.0, -7.0), p(7.0, 7.0)), 1.5, stroke, StrokeKind::Middle);
    painter.rect_stroke(Rect::from_min_max(p(-3.0, -3.0), p(3.0, 3.0)), 0.5, stroke, StrokeKind::Middle);
    for (a, b) in [((-7.0, -7.0), (-3.0, -3.0)), ((7.0, -7.0), (3.0, -3.0)), ((7.0, 7.0), (3.0, 3.0)), ((-7.0, 7.0), (-3.0, 3.0))] {
        painter.line_segment([p(a.0, a.1), p(b.0, b.1)], stroke);
    }
}

/// 加工の印の小さいもの（レイヤーの小さな絵の角に添える。外の四角と内の四角だけ）。
pub(super) fn draw_foil_small(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    let p = |x: f32, y: f32| c + vec2(x, y);
    painter.rect_stroke(Rect::from_min_max(p(-4.0, -4.0), p(4.0, 4.0)), 1.0, stroke, StrokeKind::Middle);
    painter.rect_stroke(Rect::from_min_max(p(-1.5, -1.5), p(1.5, 1.5)), 0.3, stroke, StrokeKind::Middle);
}

/// フォルダ（グループ）。
pub(super) fn draw_folder(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    let p = |x: f32, y: f32| c + vec2(x, y);
    painter.add(Shape::closed_line(vec![p(-7.0, -4.5), p(-2.5, -4.5), p(-1.0, -3.0), p(7.0, -3.0), p(7.0, 5.0), p(-7.0, 5.0)], stroke));
    painter.line_segment([p(-7.0, -1.0), p(7.0, -1.0)], stroke);
}

/// 箱（型の工程）。斜めから見た立方体。
pub(super) fn draw_box(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    let p = |x: f32, y: f32| c + vec2(x, y);
    painter.add(Shape::closed_line(vec![p(0.0, -7.0), p(6.5, -3.5), p(6.5, 4.0), p(0.0, 7.5), p(-6.5, 4.0), p(-6.5, -3.5)], stroke));
    painter.add(Shape::line(vec![p(-6.5, -3.5), p(0.0, 0.0), p(6.5, -3.5)], stroke));
    painter.line_segment([p(0.0, 0.0), p(0.0, 7.5)], stroke);
}

/// 書き出し（出力の工程）。受け皿と、上へ出ていく矢印。
pub(super) fn draw_export(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    let p = |x: f32, y: f32| c + vec2(x, y);
    painter.add(Shape::line(vec![p(-6.5, 1.0), p(-6.5, 6.5), p(6.5, 6.5), p(6.5, 1.0)], stroke));
    painter.line_segment([p(0.0, 3.0), p(0.0, -7.0)], stroke);
    painter.add(Shape::line(vec![p(-3.5, -3.5), p(0.0, -7.0), p(3.5, -3.5)], stroke));
}

/// 吹き出し（エージェント）。
pub(super) fn draw_chat(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    let p = |x: f32, y: f32| c + vec2(x, y);
    painter.add(Shape::closed_line(vec![p(-7.0, -5.5), p(7.0, -5.5), p(7.0, 3.5), p(-1.0, 3.5), p(-4.5, 6.5), p(-4.0, 3.5), p(-7.0, 3.5)], stroke));
    for x in [-3.0, 0.0, 3.0] {
        painter.circle_filled(p(x, -1.0), 0.9, stroke.color);
    }
}

/// 見る向きのアイコン。小さな立方体（斜めから見た形）に、その向きから見える面を塗る。
/// `face`: 0 斜め、1 正面、2 背面、3 左、4 右、5 上、6 下（`VIEW_PRESETS` の並び）。
pub(super) fn draw_view(painter: &egui::Painter, c: Pos2, face: usize, stroke: Stroke) {
    let p = |x: f32, y: f32| c + vec2(x * 1.2, y * 1.2);
    let (a, b, cc, d) = (p(-6.0, -2.5), p(2.5, -2.5), p(2.5, 6.0), p(-6.0, 6.0));
    let (a2, b2, c2, d2) = (p(-2.5, -6.0), p(6.0, -6.0), p(6.0, 2.5), p(-2.5, 2.5));
    // 見る面は、選択の色で塗る（どの面かが、ひと目で分かるように）。
    let fill = egui::Color32::from_rgba_unmultiplied(0x37, 0x8e, 0xf0, 210);
    let faces = [vec![], vec![a, b, cc, d], vec![a2, b2, c2, d2], vec![a, a2, d2, d], vec![b, b2, c2, cc], vec![a, a2, b2, b], vec![d, cc, c2, d2]];
    if face == 0 {
        // 斜め: 見えている 3 面を、明るさを変えて塗る。
        for (pts, alpha) in [(vec![a, a2, b2, b], 230u8), (vec![b, b2, c2, cc], 150), (vec![a, b, cc, d], 90)] {
            painter.add(Shape::convex_polygon(pts, egui::Color32::from_rgba_unmultiplied(0x37, 0x8e, 0xf0, alpha), Stroke::NONE));
        }
    } else if let Some(pts) = faces.get(face).filter(|f| !f.is_empty()) {
        painter.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
    }
    // 見えている辺は太く、隠れている辺は細く。
    let thin = Stroke::new(0.6, stroke.color.gamma_multiply(0.55));
    for (u, v) in [(d, d2), (a2, d2), (d2, c2)] {
        painter.line_segment([u, v], thin);
    }
    painter.add(Shape::closed_line(vec![a, a2, b2, c2, cc, d], stroke));
    painter.add(Shape::line(vec![a, b, cc], stroke));
    painter.line_segment([b, b2], stroke);
}

/// 箱を 90 度回すアイコン。小さな箱と、回る向きの弧の矢印。
/// `dir`: 0 左へ、1 右へ、2 奥へ（前の面が上へ）、3 手前へ（前の面が下へ）。
pub(super) fn draw_turn(painter: &egui::Painter, c: Pos2, dir: usize, stroke: Stroke) {
    let p = |x: f32, y: f32| c + vec2(x, y);
    painter.rect_stroke(Rect::from_center_size(c, vec2(6.0, 6.0)), 0.5, Stroke::new(stroke.width * 0.8, stroke.color.gamma_multiply(0.7)), StrokeKind::Middle);
    let n = 14;
    let pts: Vec<Pos2> = if dir < 2 {
        // 横回り: 箱の前を通る、横長の弧。
        (0..=n).map(|k| { let t = std::f32::consts::PI * k as f32 / n as f32; p(8.0 * t.cos(), 1.5 + 4.5 * t.sin()) }).collect()
    } else {
        // 縦回り: 箱の右を通る、縦長の弧。
        (0..=n).map(|k| { let t = std::f32::consts::PI * (k as f32 / n as f32 - 0.5); p(1.5 + 4.5 * t.cos(), 8.0 * t.sin()) }).collect()
    };
    // 矢印の向き（左へ・奥へは弧の終わり、右へ・手前へは弧の始まりに矢じり）。
    let pts: Vec<Pos2> = if dir == 1 || dir == 2 { pts.into_iter().rev().collect() } else { pts };
    let tip = pts[pts.len() - 1];
    let back = pts[pts.len() - 3];
    let along = (tip - back).normalized();
    let side = vec2(-along.y, along.x);
    painter.add(Shape::line(pts, stroke));
    painter.add(Shape::line(vec![tip - along * 3.2 + side * 2.6, tip, tip - along * 3.2 - side * 2.6], stroke));
}

/// そろえる（いちばん近い、まっすぐの向きへ）。四隅のかぎと、まん中の四角。
pub(super) fn draw_snap(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    let p = |x: f32, y: f32| c + vec2(x, y);
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        painter.add(Shape::line(vec![p(7.0 * sx, 3.5 * sy), p(7.0 * sx, 7.0 * sy), p(3.5 * sx, 7.0 * sy)], stroke));
    }
    painter.rect_stroke(Rect::from_center_size(c, vec2(6.0, 6.0)), 0.5, stroke, StrokeKind::Middle);
}

/// ごみ箱（削除）。
pub(super) fn draw_trash(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    let p = |x: f32, y: f32| c + vec2(x, y);
    painter.line_segment([p(-6.0, -4.0), p(6.0, -4.0)], stroke);
    painter.add(Shape::line(vec![p(-2.0, -4.0), p(-2.0, -6.0), p(2.0, -6.0), p(2.0, -4.0)], stroke));
    painter.add(Shape::line(vec![p(-4.5, -4.0), p(-3.8, 6.0), p(3.8, 6.0), p(4.5, -4.0)], stroke));
    painter.line_segment([p(-1.2, -1.5), p(-1.2, 4.0)], stroke);
    painter.line_segment([p(1.2, -1.5), p(1.2, 4.0)], stroke);
}

/// 行の右端の、ロックの印（押せない）。すべてをロックなら塗った鍵、一部なら線だけの鍵。
pub(super) fn lock_mark(ui: &mut egui::Ui, all: bool, tip: &str) {
    let (rect, response) = ui.allocate_exact_size(vec2(14.0, 16.0), Sense::hover());
    let color = ui.visuals().text_color().gamma_multiply(if all { 1.0 } else { 0.6 });
    let c = rect.center() + vec2(0.0, -1.0);
    let body = Rect::from_center_size(c + vec2(0.0, 2.5), vec2(8.0, 6.0));
    if all {
        ui.painter().rect_filled(body, 1.0, color);
    } else {
        ui.painter().rect_stroke(body, 1.0, Stroke::new(1.1, color), StrokeKind::Middle);
    }
    let shackle: Vec<Pos2> = (0..=10).map(|k| { let a = std::f32::consts::PI * k as f32 / 10.0; pos2(c.x - 2.4 * a.cos(), c.y - 0.5 - 2.8 * a.sin()) }).collect();
    ui.painter().add(Shape::line(shackle, Stroke::new(1.1, color)));
    response.on_hover_text(tip);
}

/// 枠のないアイコンのボタン（PhotoCraft のレイヤーの欄と同じ形）。押してあるときと、カーソルが乗ったときだけ地を塗る。
pub(super) fn bare_button(ui: &mut egui::Ui, size: egui::Vec2, on: bool, enabled: bool, tip: &str, icon: impl FnOnce(&egui::Painter, Pos2, Stroke)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let visuals = ui.visuals();
    let color = if !enabled {
        visuals.weak_text_color().gamma_multiply(0.55)
    } else if on || response.hovered() {
        visuals.strong_text_color()
    } else {
        visuals.text_color().gamma_multiply(0.85)
    };
    if on {
        ui.painter().rect_filled(rect, 3.0, if enabled { super::theme::SELECTED } else { super::theme::HOVER });
    } else if enabled && response.hovered() {
        ui.painter().rect_filled(rect, 3.0, super::theme::HOVER);
    }
    icon(ui.painter(), rect.center(), Stroke::new(1.15, color));
    response.on_hover_text(tip).on_disabled_hover_text(tip)
}

/// 山と太陽の入った四角（画像のレイヤー）。
pub(super) fn draw_image(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    let r = Rect::from_center_size(c, vec2(13.0, 11.0));
    painter.rect_stroke(r, 1.0, stroke, StrokeKind::Middle);
    painter.add(Shape::line(vec![r.left_bottom() + vec2(1.0, -1.5), c + vec2(-1.5, 0.5), c + vec2(1.0, 2.5), c + vec2(3.0, 0.8), r.right_bottom() + vec2(-1.0, -1.5)], stroke));
    painter.circle_stroke(c + vec2(2.8, -2.2), 1.3, stroke);
}

/// T の字（文字のレイヤー）。
pub(super) fn draw_type(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    painter.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, -5.0)], stroke);
    painter.line_segment([c + vec2(0.0, -5.0), c + vec2(0.0, 5.5)], stroke);
    painter.line_segment([c + vec2(-2.0, 5.5), c + vec2(2.0, 5.5)], stroke);
}

/// 四角（図形のレイヤー）。
pub(super) fn draw_square(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    painter.rect_stroke(Rect::from_center_size(c, vec2(10.0, 10.0)), 0.5, stroke, StrokeKind::Middle);
}

/// 虫めがね（絞り込み）。
pub(super) fn draw_search(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    painter.circle_stroke(c + vec2(-1.2, -1.2), 4.0, stroke);
    painter.line_segment([c + vec2(1.8, 1.8), c + vec2(5.0, 5.0)], stroke);
}

/// 三本線（欄のメニュー）。
pub(super) fn draw_menu(painter: &egui::Painter, c: Pos2, stroke: Stroke) {
    for dy in [-4.0, 0.0, 4.0] {
        painter.line_segment([c + vec2(-6.0, dy), c + vec2(6.0, dy)], stroke);
    }
}

/// 欄の見出しのタブの帯（PhotoCraft の「Layers | Channels | Paths ≡」と同じ形）。押されたタブの番号を返す。
pub(super) fn panel_tabs(ui: &mut egui::Ui, tabs: &[&str], active: usize) -> Option<usize> {
    let height = 30.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let visuals = ui.visuals().clone();
    // 帯は欄より暗く、選んでいるタブは欄と同じ色（PhotoCraft と同じ）。
    let bar = super::theme::TAB_BAR;
    ui.painter().rect_filled(rect, 0.0, bar);
    let mut x = rect.left();
    let mut clicked = None;
    for (i, label) in tabs.iter().enumerate() {
        let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), visuals.text_color());
        let w = galley.size().x + 24.0;
        let tab = Rect::from_min_size(pos2(x, rect.top()), vec2(w, height));
        let response = ui.interact(tab, ui.scope_id().with(("panel-tab", label)), Sense::click());
        if i == active {
            ui.painter().rect_filled(tab, 0.0, visuals.panel_fill);
        }
        let color = if i == active { super::theme::TEXT } else { super::theme::FAINT };
        ui.painter().galley_with_override_text_color(pos2(tab.left() + 12.0, tab.center().y - galley.size().y / 2.0), galley, color);
        if response.clicked() {
            clicked = Some(i);
        }
        x += w;
    }
    draw_menu(ui.painter(), pos2(rect.right() - 14.0, rect.center().y), Stroke::new(1.1, visuals.weak_text_color()));
    clicked
}

/// 書類のタブ（上の 1 段目）。選んでいるタブは、下の段と同じ灰色で、下の段につながって見える。
/// `closable` なら右に × を出す。返すのは、タブの反応と、× が押されたか。
pub fn doc_tab(ui: &mut egui::Ui, text: &str, active: bool, closable: bool, tip: Option<&str>) -> (egui::Response, bool) {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = if active { super::theme::TEXT } else { super::theme::MUTED };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let close_w = if closable { 18.0 } else { 0.0 };
    let size = egui::vec2(galley.size().x + 24.0 + close_w, 27.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let close_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 13.0, rect.center().y), egui::vec2(16.0, 16.0));
    let close = closable && ui.interact(close_rect, response.id.with("close"), egui::Sense::click()).on_hover_text(t!("Close this tab")).clicked();
    let fill = if active {
        super::theme::PANEL
    } else if response.hovered() {
        super::theme::TAB_BAR.gamma_multiply(1.4)
    } else {
        egui::Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, fill);
    if active {
        // 上の縁に、強調の細い線（PhotoCraft の書類のタブと同じ）。
        painter.hline(rect.x_range().shrink(3.0), rect.top() + 0.5, egui::Stroke::new(2.0, super::theme::ACCENT));
    }
    painter.galley(egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0), galley, color);
    if closable && (response.hovered() || active || ui.rect_contains_pointer(close_rect)) {
        let hot = ui.rect_contains_pointer(close_rect);
        if hot {
            painter.rect_filled(close_rect, 3.0, super::theme::HOVER);
        }
        let c = close_rect.center();
        let stroke = egui::Stroke::new(1.2, if hot { super::theme::TEXT } else { super::theme::FAINT });
        painter.line_segment([c + egui::vec2(-3.5, -3.5), c + egui::vec2(3.5, 3.5)], stroke);
        painter.line_segment([c + egui::vec2(-3.5, 3.5), c + egui::vec2(3.5, -3.5)], stroke);
    }
    let response = match tip {
        Some(tip) => response.on_hover_text(tip),
        None => response,
    };
    (response, close)
}
