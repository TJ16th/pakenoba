//! 型の線のファイル（DXF）を読み込んで、型にする。
//!
//! - 読み込みの欄: 線のまとまり（レイヤー）ごとに、切り線・折り線・ミシン目・使わない、を選ぶ。単位と裏返しも選ぶ。
//!   選ぶたびに型紙を作り直して、パネルの数と注意を出す。
//! - 型の画面: 読み込んだ型の情報、紙の厚み、折り線の角度。立体の面を押すと、その面の折り線を選べる。

use std::sync::Arc;

use eframe::egui;
use pakenoba_core::dieline::{self, Drawing, LineKind};
use pakenoba_core::template::{ImportReport, ImportSettings, Role, Template};
use pakenoba_i18n::t;

use super::{App, Dims, IMPORTED, theme};

/// 読み込みの欄の中身。
pub(super) struct DielineImport {
    name: String,
    bytes: Arc<Vec<u8>>,
    drawing: Drawing,
    settings: ImportSettings,
    /// いまの設定で作った型紙（プレビュー用）と、まとめ。作れなければ、理由。
    built: Result<(Template, ImportReport), String>,
    /// すでに読み込んだ型の読み方を変えているとき（角度は引き継ぐ）。
    editing: bool,
}

impl DielineImport {
    fn new(name: String, bytes: Arc<Vec<u8>>, drawing: Drawing, settings: ImportSettings, editing: bool) -> DielineImport {
        let built = Template::imported(&drawing, &settings, 0.3);
        DielineImport { name, bytes, drawing, settings, built, editing }
    }

    fn rebuild(&mut self) {
        self.built = Template::imported(&self.drawing, &self.settings, 0.3);
    }
}

/// AutoCAD の色の番号を、だいたいの色にする（よく使う 1〜9 だけ。ほかは灰色）。
fn aci(color: i32) -> egui::Color32 {
    match color {
        1 => egui::Color32::from_rgb(230, 60, 60),
        2 => egui::Color32::from_rgb(230, 220, 60),
        3 => egui::Color32::from_rgb(70, 200, 90),
        4 => egui::Color32::from_rgb(70, 210, 220),
        5 => egui::Color32::from_rgb(80, 120, 240),
        6 => egui::Color32::from_rgb(220, 80, 220),
        8 => egui::Color32::from_rgb(130, 130, 130),
        9 => egui::Color32::from_rgb(190, 190, 190),
        _ => egui::Color32::from_rgb(220, 220, 220),
    }
}

/// 種類ごとの、プレビューの線の色。
fn kind_color(kind: LineKind) -> egui::Color32 {
    match kind {
        LineKind::Cut => egui::Color32::from_rgb(235, 90, 90),
        LineKind::Crease => egui::Color32::from_rgb(90, 200, 120),
        LineKind::Perforation => egui::Color32::from_rgb(90, 150, 240),
        LineKind::Ignore => egui::Color32::from_gray(90),
    }
}

impl App {
    /// 「型の線のファイル（DXF）を読む…」。
    pub(super) fn choose_dieline(&mut self) {
        if let Some(path) = rfd::FileDialog::new().set_title(t!("Import a dieline (DXF)")).add_filter("DXF", &["dxf", "DXF"]).pick_file() {
            self.open_dieline(&path);
        }
    }

    /// DXF を読んで、読み込みの欄を開く。
    pub(super) fn open_dieline(&mut self, path: &std::path::Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let bytes = match std::fs::read(path) {
            Ok(bytes) => Arc::new(bytes),
            Err(e) => return self.say(t!("Could not read {}: {e}", path.display())),
        };
        match dieline::read_dxf(&bytes) {
            Ok(drawing) => {
                let settings = ImportSettings::guess(&drawing);
                self.dieline_import = Some(DielineImport::new(name, bytes, drawing, settings, false));
            }
            Err(e) => self.say(t!("Could not read {name}: {e}")),
        }
    }

    /// 読み込んだ型の読み方（線の分け方、単位、裏返し）を変える欄を開く。
    fn reopen_dieline(&mut self) {
        let Some(src) = dieline::source(self.dims.dieline) else { return };
        self.dieline_import = Some(DielineImport::new(src.name.clone(), src.bytes.clone(), src.drawing.clone(), src.settings.clone(), true));
    }

    /// 読み込みの欄。
    pub(super) fn dieline_window(&mut self, ctx: &egui::Context) {
        let Some(mut session) = self.dieline_import.take() else { return };
        let ready = session.built.is_ok();
        let accept = if session.editing { t!("Apply") } else { t!("Use this dieline") };
        let buttons = [super::dialog::Button::accept(accept, ready), super::dialog::Button::cancel(t!("Cancel"))];
        let title = t!("Import a dieline: {}", session.name);
        let mut changed = false;
        let picked = super::dialog::show(ctx, "dieline-import", &title, 720.0, &buttons, |ui| {
            ui.horizontal_top(|ui| {
                // 左: 線の分け方と単位。
                ui.vertical(|ui| {
                    ui.set_width(250.0);
                    ui.label(egui::RichText::new(t!("Lines")).strong());
                    ui.small(egui::RichText::new(t!("Choose what each layer of the file is.")).color(theme::MUTED));
                    egui::ScrollArea::vertical().max_height(230.0).auto_shrink([false, true]).show(ui, |ui| {
                        egui::Grid::new("dieline-groups").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                            for (i, group) in session.drawing.groups.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                                    ui.painter().circle_filled(dot.center(), 4.5, aci(group.color));
                                    ui.add_sized([120.0, 18.0], egui::Label::new(&group.name).truncate()).on_hover_text(t!("{} line(s)", group.count));
                                });
                                let mut kind = session.settings.kinds[i];
                                egui::ComboBox::from_id_salt(("dieline-kind", i)).width(96.0).selected_text(pakenoba_i18n::tr(kind.label())).show_ui(ui, |ui| {
                                    for k in LineKind::ALL {
                                        ui.selectable_value(&mut kind, k, pakenoba_i18n::tr(k.label()));
                                    }
                                });
                                if kind != session.settings.kinds[i] {
                                    session.settings.kinds[i] = kind;
                                    changed = true;
                                }
                                ui.end_row();
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(t!("Units")).color(theme::MUTED));
                        let units = [(1.0f32, "mm"), (10.0, "cm"), (25.4, t!("inch"))];
                        let current = units.iter().find(|(u, _)| (u - session.settings.unit_mm).abs() < 1e-4).map_or("?", |(_, n)| n);
                        egui::ComboBox::from_id_salt("dieline-unit").width(70.0).selected_text(current).show_ui(ui, |ui| {
                            for (unit, label) in units {
                                if ui.selectable_label((unit - session.settings.unit_mm).abs() < 1e-4, label).clicked() {
                                    session.settings.unit_mm = unit;
                                    changed = true;
                                }
                            }
                        });
                    });
                    if session.drawing.unit_mm.is_none() {
                        ui.small(egui::RichText::new(t!("The file does not say its units, so they were guessed from its size.")).color(theme::FAINT));
                    }
                    if ui.checkbox(&mut session.settings.mirror, t!("Drawn from the inside (flip left and right)")).on_hover_text(t!("Turn this on if the file shows the unprinted side of the sheet.")).changed() {
                        // 裏返すと、パネルの並びが変わるので、決めた角度は外す。
                        session.settings.folds.clear();
                        changed = true;
                    }
                });
                ui.add_space(12.0);
                // 右: プレビューと、結果。
                ui.vertical(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(420.0, 300.0), egui::Sense::hover());
                    let painter = ui.painter().with_clip_rect(rect);
                    painter.rect_filled(rect, 4.0, theme::SUNKEN);
                    preview(&painter, rect.shrink(12.0), &session);
                    match &session.built {
                        Ok((_, report)) => {
                            ui.label(t!("{} panels, {} folds. Flat size {:.0} × {:.0} mm.", report.panels, report.folds.len(), report.size[0], report.size[1]));
                            if report.dropped > 0 {
                                ui.small(egui::RichText::new(t!("{} piece(s) not joined by a fold were left out (waste, holes).", report.dropped)).color(theme::MUTED));
                            }
                            if report.curved > 0 {
                                ui.small(egui::RichText::new(t!("{} curved fold(s) cannot be folded yet; those panels stay flat or are left out.", report.curved)).color(theme::WARN));
                            }
                            if !report.glued {
                                ui.small(egui::RichText::new(t!("No glue flap was found. Fold angles start at 90°; you can change them afterwards.")).color(theme::MUTED));
                            }
                        }
                        Err(e) => {
                            ui.colored_label(theme::BAD, e);
                        }
                    }
                    ui.small(egui::RichText::new(t!("The cut and crease lines are written out exactly as in the file.")).color(theme::FAINT));
                });
            });
        });
        if changed {
            session.rebuild();
        }
        match picked {
            Some(0) => self.use_dieline(session),
            Some(_) => {}
            None => self.dieline_import = Some(session),
        }
    }

    /// 読み込みの欄で選んだ設定で、型にする。
    fn use_dieline(&mut self, session: DielineImport) {
        let key = match dieline::register(&session.name, session.bytes, Some(session.settings)) {
            Ok(key) => key,
            Err(e) => return self.say(e),
        };
        let dims = Dims { sides: IMPORTED, dieline: key, ..self.dims };
        if !dims.same_layout(&self.dims) && !self.decals.is_empty() {
            // 柄が外れるので、先に確かめる。
            self.pending_shape = Some(IMPORTED);
            self.pending_dims = Some((dims, self.material));
            return;
        }
        let material = self.material;
        self.apply_dims(dims, material);
        self.fold_pick = None;
    }

    /// 型の画面の右: 読み込んだ型の情報と、紙の厚み、読み方。
    pub(super) fn dieline_settings(&mut self, ui: &mut egui::Ui) {
        let Some(src) = dieline::source(self.dims.dieline) else {
            ui.colored_label(theme::BAD, t!("The dieline file is not available."));
            return;
        };
        ui.label(egui::RichText::new(&src.name).strong());
        let size = [self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]];
        ui.small(t!("Flat size: {:.0} × {:.0} mm, {} panels", size[0], size[1], self.template.panels.len()));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(t!("Paper thickness")).color(theme::MUTED));
            ui.add(egui::DragValue::new(&mut self.dims.t).range(0.1..=6.0).speed(0.01).max_decimals(2).suffix(" mm"));
        });
        ui.small(egui::RichText::new(t!("The thickness only changes how the box folds on screen. The dieline lines stay as in the file.")).color(theme::FAINT));
        ui.add_space(4.0);
        if ui.button(t!("Change how the file is read…")).on_hover_text(t!("Which layers are cut and crease lines, the units, and flipping")).clicked() {
            self.reopen_dieline();
        }
    }

    /// 型の画面の右: 折り線の角度。
    pub(super) fn fold_settings(&mut self, ui: &mut egui::Ui) {
        let Some(src) = dieline::source(self.dims.dieline) else { return };
        let report = match Template::imported(&src.drawing, &src.settings, self.dims.t) {
            Ok((_, report)) => report,
            Err(e) => {
                ui.colored_label(theme::BAD, e);
                return;
            }
        };
        ui.small(egui::RichText::new(t!("Click a panel of the box to choose its fold.")).color(theme::MUTED));
        let mut settings = src.settings.clone();
        let picked = self.fold_pick.and_then(|p| report.folds.iter().find(|f| f.panel == p).cloned());
        if let Some(fold) = &picked {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(t!("Fold of panel {}", fold.panel + 1)).strong());
            let mut angle = fold.angle;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t!("Angle")).color(theme::MUTED));
                ui.add(egui::Slider::new(&mut angle, -180.0..=180.0).step_by(1.0).suffix("°"));
            });
            ui.horizontal(|ui| {
                if ui.small_button(if angle >= 0.0 { t!("Make it a mountain fold") } else { t!("Make it a valley fold") }).clicked() {
                    angle = -angle;
                }
                if ui.add_enabled((angle - fold.guessed).abs() > 1e-3, egui::Button::new(t!("Guessed angle")).small()).on_hover_text(t!("{:.0}°", fold.guessed)).clicked() {
                    angle = fold.guessed;
                }
            });
            if (angle - fold.angle).abs() > 1e-4 {
                if (angle - fold.guessed).abs() < 1e-4 {
                    settings.folds.remove(&fold.panel);
                } else {
                    settings.folds.insert(fold.panel, angle);
                }
            }
            ui.small(egui::RichText::new(t!("A positive angle folds toward the inside (valley), a negative one toward the outside (mountain).")).color(theme::FAINT));
        }
        ui.add_space(4.0);
        egui::CollapsingHeader::new(t!("All folds")).id_salt("all-folds").show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for fold in &report.folds {
                    let text = t!("Panel {} → {}   {:.0}°", fold.parent + 1, fold.panel + 1, fold.angle);
                    let text = if (fold.angle - fold.guessed).abs() > 1e-3 { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                    if ui.selectable_label(self.fold_pick == Some(fold.panel), text).clicked() {
                        self.fold_pick = Some(fold.panel);
                    }
                }
            });
            if !src.settings.folds.is_empty() && ui.button(t!("Put all angles back to the guess")).clicked() {
                settings.folds.clear();
            }
        });
        if settings != src.settings
            && let Some(key) = dieline::with_settings(self.dims.dieline, settings)
        {
            self.dims.dieline = key;
        }
    }

    /// 立体の上で押したパネルの折り線を選ぶ（型の画面で、読み込んだ型のときだけ）。
    pub(super) fn pick_fold(&mut self, panel: Option<usize>) {
        if self.dims.sides != IMPORTED {
            return;
        }
        self.fold_pick = panel.filter(|&p| self.template.panels.get(p).is_some_and(|p| p.hinge.is_some()));
    }
}

/// 読み込みの欄のプレビュー: パネルを薄く塗り、線を種類の色で描く。
fn preview(painter: &egui::Painter, rect: egui::Rect, session: &DielineImport) {
    let (min, max) = session.drawing.bounds();
    if !(min[0].is_finite() && max[0] > min[0]) {
        return;
    }
    let unit = session.settings.unit_mm;
    let size = [(max[0] - min[0]) * unit, (max[1] - min[1]) * unit];
    let scale = (rect.width() / size[0]).min(rect.height() / size[1]);
    let origin = egui::pos2(rect.center().x - size[0] * scale * 0.5, rect.center().y + size[1] * scale * 0.5);
    // 紙の座標（左下が原点、mm）から画面へ。
    let to_screen = |p: [f32; 2]| egui::pos2(origin.x + p[0] * scale, origin.y - p[1] * scale);
    // ファイルの座標から紙の座標へ（型紙を作るときと同じ）。
    let mirror = session.settings.mirror;
    let place = |p: [f32; 2]| {
        let x = if mirror { max[0] - p[0] } else { p[0] - min[0] };
        [x * unit, (p[1] - min[1]) * unit]
    };
    if let Ok((tpl, _)) = &session.built {
        // 細い三角形の縁をなめらかにすると、とがった角から線が飛び出すので、なめらかにしない網で塗る。
        let mut mesh = egui::Mesh::default();
        for panel in &tpl.panels {
            let fill = match panel.role {
                Role::GlueTab => egui::Color32::from_rgba_unmultiplied(230, 180, 70, 40),
                Role::Face => egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26),
                _ => egui::Color32::from_rgba_unmultiplied(255, 255, 255, 12),
            };
            for tri in pakenoba_core::template::triangles(&panel.outline) {
                let base = mesh.vertices.len() as u32;
                for p in tri {
                    mesh.colored_vertex(to_screen(p), fill);
                }
                mesh.add_triangle(base, base + 1, base + 2);
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
    for path in &session.drawing.paths {
        let kind = session.settings.kinds.get(path.group).copied().unwrap_or(LineKind::Ignore);
        let points: Vec<egui::Pos2> = path.points.iter().map(|&p| to_screen(place(p))).collect();
        let width = if kind == LineKind::Ignore { 0.8 } else { 1.2 };
        painter.add(egui::Shape::line(points, egui::Stroke::new(width, kind_color(kind))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_box_writes_the_lines_of_the_file() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/dielines/pacxio/A20_20_03_01_dieline.dxf");
        let mut app = App::new(&super::super::Initial::default());
        app.open_dieline(&dir);
        let session = app.dieline_import.take().expect("the import window opens");
        assert!(session.built.is_ok());
        let cuts_in_file = session.built.as_ref().unwrap().0.drawn.as_ref().unwrap().cuts.len();
        app.decals.clear();
        app.use_dieline(session);
        app.rebuild_if_needed();
        assert_eq!(app.dims.sides, IMPORTED);
        let (cuts, folds) = crate::art::sheet_lines(&app.template);
        assert_eq!(cuts.len(), cuts_in_file);
        assert!(!folds.is_empty());
        // 開け口が見つかり、型紙の検査で問題（赤）が出ない。
        assert!(!app.template.openings.is_empty());
        assert!(app.problems.iter().all(|(blocking, _)| !blocking), "{:?}", app.problems);
        // 折り線を選んで角度を変えると、元に戻せる。
        let fold = app.template.panels.iter().position(|p| p.hinge.is_some()).unwrap();
        app.pick_fold(Some(fold));
        assert_eq!(app.fold_pick, Some(fold));
    }
}
