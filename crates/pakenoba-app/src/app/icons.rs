//! 道具のアイコン。線だけの単色で、その場で描く（アイコンのフォントや画像を持たない）。

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};

use super::Tool;

/// 道具の帯のボタン。アイコンだけなので、必ずツールチップを付ける。
pub(super) fn tool_button(ui: &mut egui::Ui, tool: Tool, selected: bool) -> egui::Response {
    strip_button(ui, tool.ready(), selected, tool.tip(), |painter, center, stroke| draw(painter, center, tool, stroke))
}

/// 道具の帯の下の、素材の欄を開け閉めするボタン。
pub(super) fn assets_button(ui: &mut egui::Ui, open: bool) -> egui::Response {
    strip_button(ui, true, open, "素材の欄を開く、閉じる", |painter, center, stroke| {
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
        Tool::Image => {
            // 額と、山と、日。
            painter.rect_stroke(Rect::from_min_max(p(3.0, 4.0), p(17.0, 16.0)), 1.5, stroke, StrokeKind::Middle);
            painter.add(Shape::line(path(&[(3.5, 14.0), (8.0, 9.5), (11.0, 12.5), (13.5, 10.5), (16.5, 13.5)]), stroke));
            painter.circle_stroke(p(13.2, 7.4), 1.2, stroke);
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
