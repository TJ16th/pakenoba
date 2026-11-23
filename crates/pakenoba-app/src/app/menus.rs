//! いちばん上のメニューバー（ファイル、編集、表示、柄、ヘルプ）。
//!
//! Windows の作法どおり、ウィンドウのいちばん上に置く。工程のタブは、その下の帯。
//! メニューは工程によって変えない（どの工程でも、すべての機能をメニューから探せる）。
//! いまの工程では使えない項目は、押せない形にして、理由をツールチップに出す。
//! 出力の項目は、押すと「出力」の工程に移り、その種類を選んだ状態にする。
//!
//! 並びは、Craft のアプリ（craft-menu）と同じ決まりにした: 「終了」はファイルの最後、
//! 「環境設定」は編集の最後、「パケノバについて」はヘルプの最後。
//! macOS の、画面の上端の OS のメニューへの対応は、まだしていない（ウィンドウの中のメニューになる）。

use eframe::egui;
use pakenoba_i18n::t;

use super::assets::Replace;
use super::select::Align;
use super::{App, View, Workspace};
use crate::paint::Picture;

/// 出力の種類の番号（`OUTPUTS` の並び）。
const OUTPUT_PSD: usize = 0;
const OUTPUT_IMAGE: usize = 1;
const OUTPUT_MOVIE: usize = 2;
const OUTPUT_PRINT: usize = 4;

/// メニューの項目 1 つ。押せたら真。
fn item(ui: &mut egui::Ui, enabled: bool, label: &str, keys: &str, why: &str) -> bool {
    let response = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys));
    let clicked = response.on_disabled_hover_text(why).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// 印つきの項目（出す、隠すなど）。押せたら真。
fn check(ui: &mut egui::Ui, enabled: bool, on: bool, label: &str, keys: &str, why: &str) -> bool {
    let mark = if on { "✓ " } else { "    " };
    item(ui, enabled, &format!("{mark}{label}"), keys, why)
}

impl App {
    /// いちばん上のメニューバー。
    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu-bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                // アプリの名前は置かない（Windows の作法。名前はウィンドウの題とアイコンにある）。
                ui.add_space(2.0);
                self.file_menu(ui);
                self.edit_menu(ui);
                self.view_menu(ui);
                self.artwork_menu(ui);
                self.help_menu(ui);
            });
        });
        self.about_window(ui.ctx());
    }

    /// 「ファイル」の中の、書き出しの項目。押すと「出力」の工程に移り、その種類を選ぶ。
    pub(super) fn export_items(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(t!("Export"), |ui| {
            ui.set_min_width(240.0);
            for (kind, label) in [(OUTPUT_PSD, t!("Dieline (PSD)…")), (OUTPUT_IMAGE, t!("16:9 image…")), (OUTPUT_MOVIE, t!("Box-opening video…")), (OUTPUT_PRINT, t!("Full-size dieline (PDF)…"))] {
                if item(ui, true, label, "", "") {
                    self.workspace = Workspace::Output;
                    self.library.open = false;
                    self.output_kind = kind;
                }
            }
        });
    }

    /// 「編集」のメニュー。
    fn edit_menu(&mut self, ui: &mut egui::Ui) {
        let design = self.workspace == Workspace::Design && !self.library.open;
        let selected = self.selected.is_some_and(|i| i < self.decals.len());
        ui.menu_button(t!("Edit"), |ui| {
            ui.set_min_width(220.0);
            if item(ui, self.history.can_undo(), t!("Undo"), "Ctrl+Z", t!("Nothing to undo")) {
                self.undo();
            }
            if item(ui, self.history.can_redo(), t!("Redo"), "Ctrl+Shift+Z", t!("Nothing to redo")) {
                self.redo();
            }
            ui.separator();
            if item(ui, design && selected, t!("Delete"), "Delete", t!("Select artwork in the Design step")) {
                self.delete_selected();
            }
            if item(ui, design && !self.decals.is_empty(), t!("Select all"), "Ctrl+A", t!("Usable in the Design step")) {
                self.select_all();
            }
            if item(ui, design && selected, t!("Deselect"), "Esc", t!("Nothing is selected")) {
                self.select_only(None);
            }
            ui.separator();
            ui.menu_button(t!("Preferences"), |ui| {
                ui.set_min_width(200.0);
                self.language_menu(ui);
            });
        });
    }

    /// 言語の選択。読めない言語にしてしまっても見つけられるように、英語の名前も添える。
    fn language_menu(&mut self, ui: &mut egui::Ui) {
        let label = if pakenoba_i18n::current() == "en" { "Language".to_owned() } else { format!("{} / Language", t!("Language")) };
        let mut choice = None;
        ui.menu_button(label, |ui| {
            if ui.radio(self.settings.language.is_none(), t!("Same as the system")).clicked() {
                choice = Some(None);
                ui.close();
            }
            for lang in &pakenoba_i18n::LANGS {
                if ui.radio(self.settings.language.as_deref() == Some(lang.code), lang.name).clicked() {
                    choice = Some(Some(lang.code.to_owned()));
                    ui.close();
                }
            }
        });
        if let Some(choice) = choice {
            self.set_language(choice);
        }
    }

    /// 「表示」のメニュー。工程の切り替えと、表示の出し方。
    fn view_menu(&mut self, ui: &mut egui::Ui) {
        let design = self.workspace == Workspace::Design && !self.library.open;
        ui.menu_button(t!("View"), |ui| {
            ui.set_min_width(240.0);
            for (workspace, label, keys) in [(Workspace::Shape, t!("Structure"), "Ctrl+Alt+1"), (Workspace::Design, t!("Design"), "Ctrl+Alt+2"), (Workspace::Output, t!("Output"), "Ctrl+Alt+3")] {
                if check(ui, true, !self.library.open && self.workspace == workspace, label, keys, "") {
                    self.go_to(workspace);
                }
            }
            if check(ui, true, self.library.open, t!("Assets screen"), "", "") {
                self.toggle_library();
            }
            ui.separator();
            let why = t!("Usable in the Design step");
            for (view, label, key) in [(View::Solid, t!("3D"), "1"), (View::Front, t!("Front"), "2"), (View::Back, t!("Back"), "3")] {
                if check(ui, design, self.shown.get(view), label, key, why) && !self.shown.toggle(view) {
                    self.say(t!("The last view cannot be hidden."));
                }
            }
            if check(ui, design, self.assets_open, t!("Assets panel"), "", why) {
                self.assets_open = !self.assets_open;
            }
            if check(ui, true, self.show_guides, t!("Guides (cut lines, fold lines, glue flaps)"), "", "") {
                self.show_guides = !self.show_guides;
                (self.art_dirty, self.back_dirty) = (true, true);
            }
            ui.separator();
            if item(ui, true, t!("Fit to window"), "Ctrl+0", "") {
                self.fit_view();
            }
        });
    }

    /// 「柄」のメニュー。デザインの工程で、柄を選んでいるときに使える。
    fn artwork_menu(&mut self, ui: &mut egui::Ui) {
        let design = self.workspace == Workspace::Design && !self.library.open;
        let index = self.selected.filter(|&i| design && i < self.decals.len());
        let count = if index.is_some() { self.selection().len() } else { 0 };
        let single = index.filter(|_| count == 1);
        let locked = index.is_some_and(|i| self.decals[i].layer.locked);
        let why = t!("Select artwork in the Design step");
        ui.menu_button(t!("Object"), |ui| {
            ui.set_min_width(260.0);
            if item(ui, single.is_some_and(|i| self.neighbor(i, 1).is_some()), t!("Bring forward"), "", why) {
                self.reorder_selected(1);
            }
            if item(ui, single.is_some_and(|i| self.neighbor(i, -1).is_some()), t!("Send backward"), "", why) {
                self.reorder_selected(-1);
            }
            if item(ui, single.is_some() && !locked, t!("Rotate 90°"), "", why) {
                self.rotate_selected();
            }
            ui.add_enabled_ui(index.is_some(), |ui| {
                ui.menu_button(t!("Align"), |ui| {
                    ui.set_min_width(200.0);
                    for (how, label) in [(Align::Left, t!("Left")), (Align::CenterX, t!("Horizontal center")), (Align::Right, t!("Right")), (Align::Top, t!("Top")), (Align::CenterY, t!("Vertical center")), (Align::Bottom, t!("Bottom"))] {
                        if item(ui, true, label, "", "") {
                            self.align(how);
                        }
                    }
                    ui.separator();
                    let three = t!("Select three or more pieces of artwork");
                    if item(ui, count >= 3, t!("Distribute horizontally"), "", three) {
                        self.distribute(true);
                    }
                    if item(ui, count >= 3, t!("Distribute vertically"), "", three) {
                        self.distribute(false);
                    }
                });
            })
            .response
            .on_disabled_hover_text(why);
            ui.separator();
            let asset = single.and_then(|i| self.asset_of(&self.decals[i].picture));
            let uses = asset.map_or(0, |a| self.uses(a));
            if item(ui, asset.is_some(), &t!("Replace asset… ({uses} place(s))"), "", t!("Select artwork that uses an asset")) {
                self.replace_request = asset.map(Replace::Asset);
            }
            if item(ui, single.is_some(), t!("Replace this artwork only…"), "", why) {
                self.replace_request = single.map(Replace::Decal);
            }
            if item(ui, single.is_some(), t!("Edit in another app"), "Ctrl+E", why) {
                self.edit_selected();
            }
            let raster = single.filter(|&i| matches!(self.decals[i].picture, Picture::Image { vector: false, .. }));
            if item(ui, raster.is_some() && !locked && self.vectorizing.is_none(), t!("Vectorize"), "", t!("Select image artwork (not vector, not locked)"))
                && let Some(i) = raster
            {
                self.start_vectorize(i);
            }
            ui.separator();
            if item(ui, index.is_some(), t!("Delete"), "Delete", why) {
                self.delete_selected();
            }
        });
    }

    /// 「ヘルプ」のメニュー。
    fn help_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(t!("Help"), |ui| {
            ui.set_min_width(200.0);
            if item(ui, true, t!("About PAKENOBA"), "", "") {
                self.about_open = true;
            }
        });
    }

    /// 「パケノバについて」の窓。
    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = true;
        egui::Window::new(t!("About PAKENOBA")).collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.heading(t!("PAKENOBA"));
            ui.label(t!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(6.0);
            ui.label(t!("Software for designing packages: choose a dieline, place artwork on the box, and export."));
            ui.add_space(6.0);
            ui.label(t!("The code is licensed under MIT or Apache-2.0, at your option. The PAKENOBA name and logo are not covered by the code license."));
            ui.add_space(6.0);
            ui.label(t!("Uses craft-libs and VectorCraft's Image Trace (MIT or Apache-2.0)."));
            ui.add_space(8.0);
            if ui.button(t!("Close")).clicked() {
                self.about_open = false;
            }
        });
        if !open {
            self.about_open = false;
        }
    }

    /// 工程を移る（「素材」の画面は閉じる）。
    pub(super) fn go_to(&mut self, workspace: Workspace) {
        self.workspace = workspace;
        self.library.open = false;
    }

    /// 表示を全体に戻す（Ctrl+0 と同じ）。
    pub(super) fn fit_view(&mut self) {
        self.zoom = 1.0;
        self.flat = [super::FlatCam::default(); 2];
    }

    /// 選んでいる柄を 90 度回す。
    pub(super) fn rotate_selected(&mut self) {
        let Some(index) = self.selected.filter(|&i| i < self.decals.len() && !self.decals[i].layer.locked) else { return };
        self.checkpoint();
        let decal = &mut self.decals[index];
        let turned = decal.angle + std::f32::consts::FRAC_PI_2;
        decal.angle = (turned + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        (self.art_dirty, self.back_dirty) = (true, true);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Initial, Tool};
    use super::*;

    #[test]
    fn 柄を回すと元に戻せる() {
        let mut app = App::new(&Initial::default());
        app.workspace = Workspace::Design;
        let at = pakenoba_core::surface::Decal { panel: 0, at: [0.5, 0.5], size: [1.0, 1.0], angle: 0.0 }.center(&app.template);
        app.add_shape_at(0, at, false);
        let index = app.decals.len() - 1;
        let before = app.decals[index].decal.angle;
        app.rotate_selected();
        assert!((app.decals[index].decal.angle - before - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        app.undo();
        assert_eq!(app.decals[index].decal.angle, before);
        // 道具は、メニューからは変わらない。
        assert!(app.tool == Tool::Select);
    }

    #[test]
    fn 工程を移ると素材の画面は閉じる() {
        let mut app = App::new(&Initial::default());
        app.library.open = true;
        app.go_to(Workspace::Output);
        assert!(!app.library.open);
        assert!(app.workspace == Workspace::Output);
    }
}
