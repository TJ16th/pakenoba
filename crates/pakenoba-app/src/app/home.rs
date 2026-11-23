//! はじめの画面（ホーム）。Photoshop のホーム画面のように、起動したら、新しい案件を始めるか、
//! 最近使った案件を開くかを選ぶ。上の帯の「ホーム」か、「ファイル」→「ホーム」で、いつでも戻れる。

use pakenoba_i18n::t;
use std::path::PathBuf;

use eframe::egui::{self, Color32, Rect, Sense, pos2, vec2};

use super::{App, SHAPES};

impl App {
    /// ホームの画面全体。
    pub(super) fn home_screen(&mut self, ui: &mut egui::Ui) {
        let mut open: Option<Option<PathBuf>> = None;
        let mut new_shape: Option<usize> = None;
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(24.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(24.0);
                    // 左: ロゴと、始め方。
                    ui.vertical(|ui| {
                        ui.set_width(300.0);
                        if self.home_logo.is_none() {
                            self.home_logo = super::splash::logo_image().map(|image| ui.ctx().load_texture("home-logo", image, egui::TextureOptions::LINEAR));
                        }
                        if let Some(logo) = &self.home_logo {
                            let size = logo.size_vec2();
                            ui.image((logo.id(), vec2(260.0, 260.0 * size.y / size.x)));
                        }
                        ui.add_space(12.0);
                        let wide = |ui: &mut egui::Ui, text: &str| ui.add_sized([260.0, 34.0], egui::Button::new(egui::RichText::new(text).size(15.0)));
                        if wide(ui, t!("New project")).on_hover_text("Ctrl+N").clicked() {
                            new_shape = Some(0);
                        }
                        ui.add_space(4.0);
                        if wide(ui, t!("Open…")).on_hover_text("Ctrl+O").clicked() {
                            open = Some(None);
                        }
                        ui.add_space(18.0);
                        ui.strong(t!("Start with a box type"));
                        ui.add_space(4.0);
                        for (sides, label) in SHAPES {
                            if ui.add_sized([260.0, 26.0], egui::Button::new(pakenoba_i18n::tr(label))).clicked() {
                                new_shape = Some(sides);
                            }
                        }
                    });
                    ui.add_space(40.0);
                    // 右: 最近使った案件。
                    ui.vertical(|ui| {
                        ui.set_max_width(640.0);
                        ui.heading(t!("Recent projects"));
                        ui.add_space(8.0);
                        if self.settings.recent.is_empty() {
                            ui.weak(t!("Projects you open or save appear here."));
                        }
                        for path in self.settings.recent.clone() {
                            if recent_row(ui, &path) {
                                open = Some(Some(path));
                            }
                        }
                        if !self.settings.recent.is_empty() {
                            ui.add_space(6.0);
                            ui.weak(t!("Missing files are shown dimmed. \"Clear list\" in File > Recent projects removes them from the list."));
                        }
                    });
                });
            });
        });
        if let Some(path) = open {
            self.request_open(path);
        }
        if let Some(sides) = new_shape {
            self.home_shape = Some(sides);
            self.request_new();
        }
        if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.home = false;
        }
    }
}

/// 最近使った案件の 1 行。名前、フォルダ、最後に変えた日時。見つからないものは薄く出し、押せない。
/// 押せる行を押したら真。
fn recent_row(ui: &mut egui::Ui, path: &std::path::Path) -> bool {
    let meta = std::fs::metadata(path).ok();
    let exists = meta.is_some();
    let width = ui.available_width().min(640.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 46.0), if exists { Sense::click() } else { Sense::hover() });
    let visuals = ui.visuals();
    if response.hovered() && exists {
        ui.painter().rect_filled(rect, 6.0, visuals.widgets.hovered.bg_fill);
    }
    let strong = if exists { visuals.strong_text_color() } else { visuals.weak_text_color() };
    let weak = visuals.weak_text_color();
    let name = path.file_stem().map_or_else(|| path.display().to_string(), |s| s.to_string_lossy().into_owned());
    // 案件の印（小さな箱の絵）。
    let icon = Rect::from_min_size(rect.min + vec2(8.0, 9.0), vec2(28.0, 28.0));
    let teal = if exists { Color32::from_rgb(0x08, 0x7e, 0x8b) } else { weak };
    ui.painter().rect_stroke(icon.shrink(4.0), 3.0, (1.4, teal), egui::StrokeKind::Middle);
    ui.painter().line_segment([pos2(icon.left() + 4.0, icon.top() + 12.0), pos2(icon.right() - 4.0, icon.top() + 12.0)], (1.4, teal));
    let font = egui::FontId::proportional(14.0);
    ui.painter().text(rect.min + vec2(46.0, 6.0), egui::Align2::LEFT_TOP, &name, font, strong);
    let folder = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    let when = meta.and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string());
    let detail = match when {
        Some(when) => format!("{when}   {folder}"),
        None => t!("Not found: {folder}"),
    };
    ui.painter().text(rect.min + vec2(46.0, 26.0), egui::Align2::LEFT_TOP, detail, egui::FontId::proportional(12.0), weak);
    let response = response.on_hover_text(path.display().to_string());
    exists && response.clicked()
}
