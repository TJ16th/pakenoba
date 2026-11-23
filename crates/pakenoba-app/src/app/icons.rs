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

/// 柄をロックするかの、鍵のボタン。
pub(super) fn lock_button(ui: &mut egui::Ui, locked: bool) -> egui::Response {
    let tip = if locked { t!("Unlock") } else { t!("Lock (cannot be grabbed on the box or the dieline)") };
    row_button(ui, locked, tip, |painter, c, stroke| {
        // 錠の本体と、つる。外しているときは、つるを片側だけ浮かせる。
        painter.rect_stroke(Rect::from_center_size(c + vec2(0.0, 2.2), vec2(9.0, 6.5)), 1.0, stroke, StrokeKind::Middle);
        let lift = if locked { 0.0 } else { 2.0 };
        let shackle: Vec<Pos2> = (0..=10).map(|k| { let a = std::f32::consts::PI * k as f32 / 10.0; pos2(c.x - 2.8 * a.cos(), c.y - 1.0 - lift - 3.3 * a.sin()) }).collect();
        let (left, right) = (shackle[0], shackle[10]);
        painter.add(Shape::line(shackle, stroke));
        painter.line_segment([left, pos2(left.x, c.y - 1.0)], stroke);
        if locked {
            painter.line_segment([right, pos2(right.x, c.y - 1.0)], stroke);
        }
    })
}

/// 幅と高さの比率をそろえるボタン（Photoshop の鎖のボタン）。つないでいるあいだは、色を付けて鎖をつなげて描く。
pub(super) fn link_button(ui: &mut egui::Ui, linked: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    let visuals = ui.visuals();
    let (fill, color) = if linked {
        (visuals.selection.bg_fill, Color32::WHITE)
    } else if response.hovered() {
        (visuals.widgets.hovered.bg_fill, visuals.strong_text_color())
    } else {
        (Color32::TRANSPARENT, visuals.text_color())
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, fill);
    let c = rect.center();
    let stroke = Stroke::new(1.3, color);
    // 縦に並んだ 2 つの輪。つないでいないときは、間をあける。
    let gap = if linked { 0.0 } else { 2.0 };
    painter.rect_stroke(Rect::from_center_size(c + vec2(0.0, -3.5 - gap), vec2(6.0, 8.0)), 3.0, stroke, StrokeKind::Middle);
    painter.rect_stroke(Rect::from_center_size(c + vec2(0.0, 3.5 + gap), vec2(6.0, 8.0)), 3.0, stroke, StrokeKind::Middle);
    response.on_hover_text(if linked { t!("Width and height keep their ratio (click to change them separately)") } else { t!("Width and height change separately (click to keep their ratio)") })
}
