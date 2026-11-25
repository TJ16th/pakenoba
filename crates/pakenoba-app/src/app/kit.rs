//! プロパティの欄の部品。PhotoCraft の「Compact」の寸法（craft-libs の `theme/sizes/compact.json`）に合わせる。
//!
//! - 文字: 本文 12 px、補足 10.5 px、節の見出し 13 px（明るい色）。
//! - 入力の欄の高さ 24 px、角の丸み 3 px。行の間 4 px、節の間 12 px。
//! - 節: 上に区切りの線、左に開け閉めの三角、見出し。中身は字下げしない。

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, vec2};

use super::theme;

/// 入力の欄の高さ。
pub const CONTROL: f32 = 24.0;
/// 節の見出しの行の高さ。
const SECTION_ROW: f32 = 26.0;

/// この `ui` の中を、Compact の文字と間隔にする。
pub fn compact(ui: &mut egui::Ui) {
    use egui::{FontFamily, FontId, TextStyle};
    let style = ui.style_mut();
    for (text_style, size) in [(TextStyle::Body, 12.0), (TextStyle::Button, 12.0), (TextStyle::Small, 10.5), (TextStyle::Heading, 13.0)] {
        style.text_styles.insert(text_style, FontId::new(size, FontFamily::Proportional));
    }
    style.spacing.item_spacing = vec2(6.0, 4.0);
    style.spacing.interact_size.y = CONTROL - 4.0;
    style.spacing.button_padding = vec2(6.0, 2.0);
}

/// 開け閉めできる節。`force_open` なら、閉じていても開く（ダブルクリックで選んだパスの設定など）。
/// 開いているときだけ `body` を呼ぶ。
pub fn section<R>(ui: &mut egui::Ui, id: &str, title: &str, default_open: bool, force_open: bool, body: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let key = egui::Id::unique(("section", id));
    let mut open = ui.ctx().data_mut(|d| *d.get_persisted_mut_or(key, default_open));
    if force_open && !open {
        open = true;
        ui.ctx().data_mut(|d| d.insert_persisted(key, true));
    }
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, SECTION_ROW), Sense::click());
    let painter = ui.painter();
    // 上の区切りの線（パネルの端から端まで）。
    painter.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, theme::DIVIDER));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let color = if response.hovered() { theme::TEXT } else { theme::MUTED };
    chevron(painter, Pos2::new(rect.left() + 6.0, rect.center().y + 1.0), open, color);
    let galley = painter.layout_no_wrap(title.to_string(), egui::FontId::proportional(13.0), theme::TEXT);
    painter.galley(Pos2::new(rect.left() + 16.0, rect.center().y + 1.0 - galley.size().y / 2.0), galley, theme::TEXT);
    if response.clicked() {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_persisted(key, open));
    }
    if !open {
        return None;
    }
    ui.add_space(2.0);
    let inner = body(ui);
    ui.add_space(8.0);
    Some(inner)
}

/// 節が開いているか（節の中身のあとに、続けて足すときに使う）。
pub fn section_open(ui: &egui::Ui, id: &str, default_open: bool) -> bool {
    ui.ctx().data_mut(|d| *d.get_persisted_mut_or(egui::Id::unique(("section", id)), default_open))
}

/// 開け閉めしない見出し（区切りの線と、見出しの字だけ）。
pub fn heading(ui: &mut egui::Ui, title: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, SECTION_ROW), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, theme::DIVIDER));
    let galley = ui.painter().layout_no_wrap(title.to_string(), egui::FontId::proportional(13.0), theme::TEXT);
    ui.painter().galley(Pos2::new(rect.left(), rect.center().y + 1.0 - galley.size().y / 2.0), galley, theme::TEXT);
}

/// 開け閉めの三角（開いていれば下向き）。
fn chevron(painter: &egui::Painter, c: Pos2, open: bool, color: Color32) {
    let s = 3.0;
    let points = if open { vec![c + vec2(-s, -s * 0.6), c + vec2(s, -s * 0.6), c + vec2(0.0, s * 0.8)] } else { vec![c + vec2(-s * 0.6, -s), c + vec2(-s * 0.6, s), c + vec2(s * 0.8, 0.0)] };
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// 補足の字（10.5 px、薄い色）。
pub fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(10.5).color(theme::FAINT));
}

/// 欄の左の小さな字（W、H、X、Y）。左右に引くと、値が変わる（PhotoCraft と同じ）。
/// 返り値は、引いて値が変わったか。
pub fn glyph(ui: &mut egui::Ui, text: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, speed: f32, tip: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(14.0, CONTROL), Sense::drag());
    let response = response.on_hover_text(tip);
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let color = if response.hovered() || response.dragged() { theme::TEXT } else { theme::FAINT };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(11.5), color);
    if response.dragged() {
        let delta = response.drag_delta().x * speed;
        if delta != 0.0 {
            *value = (*value + delta).clamp(*range.start(), *range.end());
            return true;
        }
    }
    false
}

/// 回転の印（欄の左に置く、引くと角度が変わる）。
pub fn rotate_glyph(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, tip: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(14.0, CONTROL), Sense::drag());
    let response = response.on_hover_text(tip);
    let hot = response.hovered() || response.dragged();
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let color = if hot { theme::TEXT } else { theme::FAINT };
    let c = rect.center();
    let stroke = Stroke::new(1.2, color);
    // 4 分の 3 の輪と、先の矢じり。
    let r = 4.5;
    let points: Vec<Pos2> = (0..=18).map(|k| {
        let a = -std::f32::consts::FRAC_PI_2 + k as f32 / 18.0 * std::f32::consts::PI * 1.5;
        c + vec2(a.cos(), a.sin()) * r
    }).collect();
    let tip_at = *points.last().unwrap_or(&c);
    ui.painter().add(Shape::line(points, stroke));
    ui.painter().add(Shape::convex_polygon(vec![tip_at + vec2(-2.4, -0.6), tip_at + vec2(1.8, -2.6), tip_at + vec2(1.2, 2.0)], color, Stroke::NONE));
    if response.dragged() {
        let delta = response.drag_delta().x * 0.5;
        if delta != 0.0 {
            *value = (*value + delta).clamp(*range.start(), *range.end());
            return true;
        }
    }
    false
}

/// 数値の欄（幅を決め、高さは Compact の 24 px）。
pub fn number(ui: &mut egui::Ui, width: f32, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str) -> egui::Response {
    ui.add_sized([width, CONTROL], egui::DragValue::new(value).range(range).speed(0.5).max_decimals(1).suffix(unit))
}

/// 縦横の比をつなぐ鎖（目立たない線の印。つないでいるときだけ明るい色）。
pub fn link(ui: &mut egui::Ui, linked: bool, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(16.0, CONTROL), Sense::click());
    let hot = response.hovered();
    if hot {
        ui.painter().rect_filled(rect.shrink2(vec2(0.0, 3.0)), 3.0, theme::HOVER);
    }
    let color = if linked { theme::ACCENT } else if hot { theme::TEXT } else { theme::FAINT };
    let stroke = Stroke::new(1.2, color);
    let c = rect.center();
    let gap = if linked { 0.0 } else { 1.5 };
    ui.painter().rect_stroke(Rect::from_center_size(c + vec2(0.0, -3.0 - gap), vec2(5.0, 7.0)), 2.5, stroke, StrokeKind::Middle);
    ui.painter().rect_stroke(Rect::from_center_size(c + vec2(0.0, 3.0 + gap), vec2(5.0, 7.0)), 2.5, stroke, StrokeKind::Middle);
    response.on_hover_text(tip)
}

/// いくつかから 1 つを選ぶ、つながった切り替え（PhotoCraft の Segmented）。返り値は、選び直したか。
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str, &str)]) -> bool {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL), Sense::hover());
    ui.painter().rect(rect, 3.0, theme::SUNKEN, Stroke::new(1.0, theme::CONTROL_EDGE), StrokeKind::Inside);
    let each = rect.width() / options.len().max(1) as f32;
    let mut changed = false;
    for (k, (option, label, tip)) in options.iter().enumerate() {
        let cell = Rect::from_min_size(rect.min + vec2(each * k as f32, 0.0), vec2(each, rect.height())).shrink(2.0);
        let response = ui.interact(cell, ui.scope_id().with(("segment", k)), Sense::click()).on_hover_text(*tip);
        let on = *value == *option;
        let fill = if on { theme::SELECTED } else if response.hovered() { theme::HOVER } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(cell, 2.0, fill);
        let color = if on { theme::TEXT } else { theme::MUTED };
        ui.painter().text(cell.center(), egui::Align2::CENTER_CENTER, *label, egui::FontId::proportional(12.0), color);
        if response.clicked() && !on {
            *value = *option;
            changed = true;
        }
    }
    changed
}

/// 整列の印の種類。
#[derive(Clone, Copy)]
pub enum AlignIcon {
    Left,
    CenterX,
    Right,
    Top,
    CenterY,
    Bottom,
    SpreadX,
    SpreadY,
}

/// 整列の印のボタン（24 × 24）。
pub fn align_button(ui: &mut egui::Ui, icon: AlignIcon, enabled: bool, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(CONTROL, CONTROL), if enabled { Sense::click() } else { Sense::hover() });
    let response = response.on_hover_text(tip);
    if enabled && response.hovered() {
        ui.painter().rect_filled(rect, 3.0, if response.is_pointer_button_down_on() { theme::PRESSED } else { theme::HOVER });
    }
    let color = if !enabled { theme::FAINT.gamma_multiply(0.6) } else if response.hovered() { theme::TEXT } else { theme::MUTED };
    draw_align(ui.painter(), rect.center(), icon, color);
    response
}

/// 整列の印。線（そろえる先）と、長さの違う 2 本の棒。
fn draw_align(painter: &egui::Painter, c: Pos2, icon: AlignIcon, color: Color32) {
    let line = Stroke::new(1.2, color);
    let bar = |r: Rect| painter.rect_filled(r, 1.0, color);
    let s = 6.0;
    match icon {
        AlignIcon::Left | AlignIcon::CenterX | AlignIcon::Right => {
            let x = match icon {
                AlignIcon::Left => c.x - s,
                AlignIcon::Right => c.x + s,
                _ => c.x,
            };
            painter.vline(x, (c.y - s - 1.0)..=(c.y + s + 1.0), line);
            for (dy, len) in [(-3.0, 10.0), (3.0, 6.0)] {
                let left = match icon {
                    AlignIcon::Left => x + 1.5,
                    AlignIcon::Right => x - 1.5 - len,
                    _ => x - len / 2.0,
                };
                bar(Rect::from_min_size(Pos2::new(left, c.y + dy - 1.75), vec2(len, 3.5)));
            }
        }
        AlignIcon::Top | AlignIcon::CenterY | AlignIcon::Bottom => {
            let y = match icon {
                AlignIcon::Top => c.y - s,
                AlignIcon::Bottom => c.y + s,
                _ => c.y,
            };
            painter.hline((c.x - s - 1.0)..=(c.x + s + 1.0), y, line);
            for (dx, len) in [(-3.0, 10.0), (3.0, 6.0)] {
                let top = match icon {
                    AlignIcon::Top => y + 1.5,
                    AlignIcon::Bottom => y - 1.5 - len,
                    _ => y - len / 2.0,
                };
                bar(Rect::from_min_size(Pos2::new(c.x + dx - 1.75, top), vec2(3.5, len)));
            }
        }
        AlignIcon::SpreadX => {
            painter.vline(c.x - s - 1.0, (c.y - s)..=(c.y + s), line);
            painter.vline(c.x + s + 1.0, (c.y - s)..=(c.y + s), line);
            bar(Rect::from_center_size(c, vec2(3.5, 8.0)));
        }
        AlignIcon::SpreadY => {
            painter.hline((c.x - s)..=(c.x + s), c.y - s - 1.0, line);
            painter.hline((c.x - s)..=(c.x + s), c.y + s + 1.0, line);
            bar(Rect::from_center_size(c, vec2(8.0, 3.5)));
        }
    }
}
