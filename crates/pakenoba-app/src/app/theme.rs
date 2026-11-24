//! 画面の色。PhotoCraft の「Pro」の暗いテーマ（craft-libs の `theme/themes/pro.json`）と同じ値にする。
//!
//! 考え方も PhotoCraft と同じ: 欄（パネル）は明るめの灰色の箱で、箱と箱の間と外は、いちばん暗い灰色の溝。
//! 入力の欄はパネルより沈んだ色、選んだ行は少し明るい灰色、強調だけが青。

use eframe::egui::{self, Color32, CornerRadius, Stroke};

/// パネルの間と外の溝（surface.background）。
pub const BACKGROUND: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
/// 絵を置く場所（surface.canvas）。
pub const CANVAS: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
/// パネル（surface.panel）。
pub const PANEL: Color32 = Color32::from_rgb(0x32, 0x32, 0x32);
/// パネルの見出しのタブの帯。
pub const TAB_BAR: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
/// 入力の欄（surface.sunken）。
pub const SUNKEN: Color32 = Color32::from_rgb(0x24, 0x24, 0x24);
/// レイヤーの目の列（パネルより少し暗い）。
pub const EYE_COLUMN: Color32 = Color32::from_rgb(0x2d, 0x2d, 0x2d);
/// カーソルが乗ったとき（state.hover）、押したとき（state.pressed）、選んだとき（state.selected）。
pub const HOVER: Color32 = Color32::from_rgb(0x42, 0x42, 0x42);
pub const PRESSED: Color32 = Color32::from_rgb(0x4e, 0x4e, 0x4e);
pub const SELECTED: Color32 = Color32::from_rgb(0x52, 0x52, 0x52);
/// 部品のふち（line.control_edge）と、区切りの線（line.divider）。
pub const CONTROL_EDGE: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
pub const DIVIDER: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
/// 行の間の、ごく薄い線。
pub const ROW_LINE: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
/// 文字（text.normal、text.muted、text.faint）。
pub const TEXT: Color32 = Color32::from_rgb(0xde, 0xde, 0xde);
pub const MUTED: Color32 = Color32::from_rgb(0xb2, 0xb2, 0xb2);
pub const FAINT: Color32 = Color32::from_rgb(0x8c, 0x8c, 0x8c);
/// 強調（accent.solid）。選んでいる切り替え、チェック、主なボタン。
pub const ACCENT: Color32 = Color32::from_rgb(0x37, 0x8e, 0xf0);
/// 輪郭だけのボタン（「初期設定に戻す」など）のふち。
pub const OUTLINE: Color32 = Color32::from_rgb(0x8b, 0x8b, 0x8b);

/// egui の見た目を、上の色にする。
pub fn apply(style: &mut egui::Style) {
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, BACKGROUND);
    v.extreme_bg_color = SUNKEN;
    v.text_edit_bg_color = Some(SUNKEN);
    v.faint_bg_color = EYE_COLUMN;
    v.code_bg_color = SUNKEN;
    v.override_text_color = None;
    v.weak_text_color = Some(FAINT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);

    let radius = CornerRadius::same(3);
    let w = &mut v.widgets;
    // 文字だけの部品（ラベルなど）。
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, DIVIDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.corner_radius = radius;
    // 押せる部品（ボタン、入力の欄、選択の箱）。PhotoCraft と同じく、沈んだ地で、ふちは目立たせない。
    w.inactive.bg_fill = SUNKEN;
    w.inactive.weak_bg_fill = SUNKEN;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;
    w.hovered.bg_fill = HOVER;
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, CONTROL_EDGE);
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = radius;
    w.active.bg_fill = PRESSED;
    w.active.weak_bg_fill = PRESSED;
    w.active.bg_stroke = Stroke::new(1.0, CONTROL_EDGE);
    w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.active.corner_radius = radius;
    w.open.bg_fill = PRESSED;
    w.open.weak_bg_fill = PRESSED;
    w.open.bg_stroke = Stroke::new(1.0, CONTROL_EDGE);
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = radius;
}

/// パネルの箱（明るめの灰色。まわりの溝との間に少しすき間）。
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new().fill(PANEL).inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 8 }).corner_radius(CornerRadius::same(2))
}

/// パネルの中身の余白。
pub fn panel_body() -> egui::Frame {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6))
}

/// 輪郭だけの丸いボタン（PhotoCraft の「Reset to defaults」と同じ形）。幅いっぱい。
pub fn pill_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let button = egui::Button::new(egui::RichText::new(text).color(TEXT)).fill(PANEL).stroke(Stroke::new(1.0, OUTLINE)).corner_radius(14.0);
    ui.add_sized([ui.available_width(), 28.0], button)
}
