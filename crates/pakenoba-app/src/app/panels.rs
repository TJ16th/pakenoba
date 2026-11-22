//! 画面の部品。上のタブ、上の帯、左の道具と一覧、右の設定、下の帯。
//!
//! 役割は工程をまたいで変えない。左は「選ぶもの・道具」、右は「選んだものの設定」、下は「動き」。

use std::path::PathBuf;

use eframe::egui;

use super::{App, PILLOW, SHAPES, Tool, View, Workspace, icons};
use crate::paint::Picture;


/// 出力の種類。（名前、もう使えるか）。
use super::outputs::OUTPUTS;

/// 数値の欄。右にそろえ、単位を必ず添える。
fn number(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str) -> egui::Response {
    ui.add_sized([96.0, ui.spacing().interact_size.y], egui::DragValue::new(value).range(range).speed(0.5).max_decimals(1).suffix(format!(" {unit}")))
}

impl App {
    /// 上: 工程の切り替え。左から順に進めば完成する。
    pub(super) fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("workspaces").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.strong("パケノバ");
                ui.add_space(8.0);
                self.file_menu(ui);
                ui.add_space(8.0);
                for (workspace, label) in [(Workspace::Shape, "型"), (Workspace::Design, "デザイン"), (Workspace::Output, "出力")] {
                    // 「素材」の画面を開いているあいだは、どの工程も選んでいない見た目にする。
                    if ui.selectable_label(!self.library.open && self.workspace == workspace, format!("  {label}  ")).clicked() {
                        self.workspace = workspace;
                        self.library.open = false;
                    }
                }
                // 案件の名前と、保存していない変更の印。
                ui.add_space(16.0);
                let unsaved = self.unsaved();
                let name = ui.label(format!("{}{}", self.project_name(), if unsaved { " *" } else { "" }));
                if self.is_saving() {
                    ui.weak("保存しています…");
                } else if unsaved {
                    name.on_hover_text("保存していない変更があります（Ctrl+S で保存）");
                }
                // メニューバーを作るまでの、元に戻すとやり直すの置き場。
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    if ui.selectable_label(self.library.open, "  素材  ").on_hover_text("素材を探して、この案件に取り込む画面（もう一度押すと戻ります）").clicked() {
                        self.toggle_library();
                    }
                    ui.separator();
                    if ui.add_enabled(self.history.can_redo(), egui::Button::new("やり直す")).on_hover_text("Ctrl+Shift+Z").clicked() {
                        self.redo();
                    }
                    if ui.add_enabled(self.history.can_undo(), egui::Button::new("元に戻す")).on_hover_text("Ctrl+Z").clicked() {
                        self.undo();
                    }
                });
            });
        });
    }

    /// 上の帯: 選んだ道具の設定。右端の表示の切り替えは、道具に関係なく常に出す。
    pub(super) fn options_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("options").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(ui.spacing().interact_size.y + 4.0);
                ui.add_space(4.0);
                match self.tool {
                    Tool::Select => match self.selected.filter(|&i| i < self.decals.len()) {
                        Some(_) if self.selection().len() > 1 => {
                            // いくつも選んでいるとき: 整列と、まとめて消す。
                            ui.label(format!("{} 個の柄", self.selection().len()));
                            ui.separator();
                            ui.label("そろえる");
                            self.align_buttons(ui);
                            ui.separator();
                            if ui.button("消す").on_hover_text("Delete").clicked() {
                                self.delete_selected();
                            }
                        }
                        Some(index) => {
                            ui.label(self.layer_label(index));
                            ui.separator();
                            ui.label("重なり順");
                            if ui.add_enabled(self.neighbor(index, 1).is_some(), egui::Button::new("前面へ")).clicked() {
                                self.reorder_selected(1);
                            }
                            if ui.add_enabled(self.neighbor(index, -1).is_some(), egui::Button::new("背面へ")).clicked() {
                                self.reorder_selected(-1);
                            }
                            ui.separator();
                            if ui.add_enabled(!self.decals[index].layer.locked, egui::Button::new("90 度回す")).on_disabled_hover_text("ロックしています").clicked() {
                                self.checkpoint();
                                let decal = &mut self.decals[index];
                                let turned = decal.angle + std::f32::consts::FRAC_PI_2;
                                decal.angle = (turned + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                                self.art_dirty = true;
                                self.back_dirty = true;
                            }
                            ui.separator();
                            ui.label("パネルに");
                            self.align_buttons(ui);
                        }
                        None => {
                            ui.weak("柄をクリックして選びます。Shift を押しながらで、いくつも選べます（Ctrl+A ですべて）。素材は、左の素材の欄から、箱や展開図へドラッグして置きます");
                        }
                    },
                    Tool::Hand => {
                        ui.weak("ドラッグで、箱を回す、展開図をずらす");
                    }
                    Tool::Text => self.text_bar(ui),
                    Tool::Shape | Tool::Ai => {}
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    // 右から並べるので、逆の順に足す。
                    for (view, label, key) in [(View::Back, "裏", "3"), (View::Front, "表", "2"), (View::Solid, "立体", "1")] {
                        let response = ui.selectable_label(self.shown.get(view), format!(" {label} ")).on_hover_text(format!("{label}の表示を出す、隠す。{key} のキーか、Alt を押しながら押すと、この表示だけにする"));
                        if response.clicked() {
                            if ui.input(|i| i.modifiers.alt) {
                                self.shown.solo(view);
                            } else if !self.shown.toggle(view) {
                                self.say("最後の 1 つの表示は隠せません。");
                            }
                        }
                    }
                    ui.weak("表示");
                });
            });
        });
    }

    /// 左（デザイン）: 道具の帯。アイコンだけ。
    pub(super) fn tool_strip(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("tool-strip").resizable(false).exact_size(42.0).show(ui, |ui| {
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for tool in Tool::ALL {
                if icons::tool_button(ui, tool, self.tool == tool).clicked() {
                    self.tool = tool;
                }
            }
            ui.add_space(6.0);
            ui.separator();
            if icons::assets_button(ui, self.assets_open).clicked() {
                self.assets_open = !self.assets_open;
            }
        });
    }

    /// 右（デザイン）: 選んだ柄の設定と、レイヤー。
    pub(super) fn design_side(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("design-side").resizable(false).exact_size(280.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(4.0);
                // 型の工程へ戻る入口。
                ui.horizontal(|ui| {
                    if ui.small_button("型").on_hover_text("型の工程へ戻って、形と寸法を変える").clicked() {
                        self.workspace = Workspace::Shape;
                    }
                    ui.weak(self.shape_summary());
                });
                ui.separator();
                self.properties(ui);
                ui.add_space(6.0);
                ui.separator();
                self.layers(ui);
            });
        });
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        ui.strong("プロパティ");
        let Some(index) = self.selected.filter(|&i| i < self.decals.len()) else {
            ui.weak("柄を選ぶと、ここに設定が出ます。");
            return;
        };
        // 値を変えたときに、変える前へ戻せるよう、いまの状態を取っておく。
        let others = self.selection().len() - 1;
        if others > 0 {
            ui.weak(format!("ほかに {others} 個を選んでいます。動かすと、いっしょに動きます。"));
        }
        let locked = self.decals[index].layer.locked;
        if locked {
            ui.weak("ロックしています。動かすには、レイヤーの欄で鍵を外してください。");
        }
        self.text_properties(ui, index, locked);
        let earlier = self.snapshot();
        let template = &self.template;
        let decal = &mut self.decals[index];
        let before = decal.decal;
        let band = matches!(decal.picture, Picture::Band);
        let was_back = decal.back;
        let mut back = was_back;
        // 位置は、展開図の左上からの距離（表や裏の表示の定規と同じ向き）。
        // 裏の柄は、裏の表示（表を左右に裏返した向き）で見たときの値にする。
        let sheet_w = template.max[0] - template.min[0];
        let mirror = move |x: f32| if was_back { sheet_w - x } else { x };
        let turn = if was_back { -1.0 } else { 1.0 };
        let center = decal.center(template);
        let (x0, y0) = (mirror(center[0] - template.min[0]), template.max[1] - center[1]);
        let (mut x, mut y) = (x0, y0);
        let mut width = decal.size[0];
        let mut height = decal.size[1];
        // 0 に負の符号が付かないよう、0 を足しておく（「-0.0 度」と出るのを防ぐ）。
        let mut degrees = decal.angle.to_degrees() * turn + 0.0;
        ui.add_enabled_ui(!locked, |ui| egui::Grid::new("properties").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("種類");
            ui.label(decal.picture.label());
            ui.end_row();
            ui.label("紙の面");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut back, false, " 表 ").on_hover_text("紙の表（箱の外側）に置く");
                ui.selectable_value(&mut back, true, " 裏 ").on_hover_text("紙の裏（箱の内側）に置く。フタの裏など");
            });
            ui.end_row();
            ui.label("位置 X");
            number(ui, &mut x, -2000.0..=4000.0, "mm");
            ui.end_row();
            ui.label("位置 Y");
            number(ui, &mut y, -2000.0..=4000.0, "mm");
            ui.end_row();
            ui.label("幅");
            number(ui, &mut width, 2.0..=2000.0, "mm");
            ui.end_row();
            ui.label("高さ");
            number(ui, &mut height, 2.0..=2000.0, "mm");
            ui.end_row();
            ui.label("回転");
            number(ui, &mut degrees, -180.0..=180.0, "度");
            ui.end_row();
        }));
        if (x, y) != (x0, y0) {
            // 数値で動かすときは、置かれている面の中にとどめる（隣の面へは、ドラッグで動かす）。
            decal.set_center(template, [template.min[0] + mirror(x), template.max[1] - y]);
            decal.at = [decal.at[0].clamp(0.0, 1.0), decal.at[1].clamp(0.0, 1.0)];
        }
        if band {
            decal.size = [width, height];
        } else if width != decal.size[0] {
            // ロゴと画像は、縦横の比を保つ。
            decal.size = [width, decal.size[1] * width / decal.size[0]];
        } else if height != decal.size[1] {
            decal.size = [decal.size[0] * height / decal.size[1], height];
        }
        decal.angle = (degrees * turn).to_radians();
        let moved_side = back != decal.back;
        if moved_side {
            // 表と裏を入れ替える。場所は同じ（ちょうど裏側）。見た目の向きが変わらないよう、回転は逆にする。
            decal.back = back;
            decal.angle = -decal.angle;
        }
        if before != decal.decal || moved_side {
            self.art_dirty = true;
            // スライドや入力のひと続きは、1 回ぶんにまとめる。面の入れ替えは、いつも 1 回ぶん。
            if !self.editing || moved_side {
                self.history.record(earlier);
            }
            self.editing = !moved_side;
        }
        if moved_side {
            self.back_dirty = true;
            self.grab = None;
            self.say(if back { "紙の裏へ移しました。「裏」の表示か、箱を開けた内側で見られます。" } else { "紙の表へ移しました。" });
        }
        // どの面にかかっているか。
        let pieces = self.surface.layout(&self.template, &self.decals[index]);
        let mut names: Vec<&str> = pieces.iter().map(|p| self.template.panels[p.panel].name).collect();
        names.dedup();
        let seams = pieces.iter().map(|p| p.seams).max().unwrap_or(0);
        ui.add_space(2.0);
        ui.small(format!("かかっている面: {}", names.join("、")));
        if seams > 0 {
            ui.small(format!("合わせ目を {seams} 回またいでいます。"));
        }
        ui.add_space(2.0);
        if ui.add_enabled(!locked, egui::Button::new("この柄を消す")).on_hover_text("Delete").on_disabled_hover_text("ロックしている柄は消せません").clicked() {
            self.delete_selected();
        }
    }

    fn layers(&mut self, ui: &mut egui::Ui) {
        ui.strong("レイヤー");
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            if ui.checkbox(&mut self.show_guides, "ガイド（切り線、折り線、のりしろ）").changed() {
                self.art_dirty = true;
                self.back_dirty = true;
            }
            // 紙の表と裏に分けて、上に重なっているものから並べる。
            // 行ごとに、目（見せる、隠す）、鍵（ロック）、名前。名前をダブルクリックで名前を変える。行をドラッグで重なり順を変える。
            let mut rows_at: Vec<(usize, egui::Rect)> = Vec::new();
            for back in [false, true] {
                ui.label(if back { "裏（箱の内側）" } else { "表（箱の外側）" });
                let rows: Vec<usize> = (0..self.decals.len()).rev().filter(|&i| self.decals[i].back == back).collect();
                if rows.is_empty() && self.bases[back as usize].is_none() {
                    ui.small("　柄はありません");
                }
                for index in rows {
                    let row = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let layer = self.decals[index].layer.clone();
                        if icons::eye_button(ui, !layer.hidden).clicked() {
                            self.checkpoint();
                            self.decals[index].layer.hidden = !layer.hidden;
                            (self.art_dirty, self.back_dirty) = (true, true);
                        }
                        if icons::lock_button(ui, layer.locked).clicked() {
                            self.checkpoint();
                            self.decals[index].layer.locked = !layer.locked;
                            self.grab = None;
                        }
                        ui.add_space(4.0);
                        if let Some((renamed, text)) = &mut self.renaming
                            && *renamed == index
                        {
                            let edit = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                            if !edit.has_focus() && !edit.lost_focus() {
                                edit.request_focus();
                            }
                            let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                            if cancel || edit.lost_focus() {
                                let text = text.trim().to_owned();
                                self.renaming = None;
                                if !cancel {
                                    let name = (!text.is_empty()).then_some(text);
                                    if name != self.decals[index].layer.name {
                                        self.checkpoint();
                                        self.decals[index].layer.name = name;
                                    }
                                }
                            }
                            return None;
                        }
                        let label = self.layer_label(index);
                        let text = if layer.hidden { egui::RichText::new(label).weak() } else { egui::RichText::new(label) };
                        let selected = self.is_selected(index);
                        let response = ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| ui.selectable_label(selected, text)).inner.interact(egui::Sense::click_and_drag());
                        Some(response)
                    });
                    rows_at.push((index, row.response.rect));
                    let Some(response) = row.inner else { continue };
                    if response.hovered() {
                        self.hover_now = Some(index);
                    }
                    if response.double_clicked() {
                        self.renaming = Some((index, self.decals[index].layer.name.clone().unwrap_or_default()));
                    } else if response.clicked() {
                        // Shift を押していれば、足すか外す。押していなければ、その柄だけを選ぶ（もう一度押しても外さない。
                        // ダブルクリックで名前を変えるときに、表示がずれないように）。
                        let shift = ui.input(|i| i.modifiers.shift);
                        self.select_click(Some(index), shift);
                    }
                    if response.secondary_clicked() && !self.is_selected(index) {
                        self.select_only(Some(index));
                    }
                    if response.drag_started() {
                        self.layer_drag = Some(index);
                    }
                    let response = response.on_hover_text("ダブルクリックで名前を変える。ドラッグで重なり順を変える");
                    response.context_menu(|ui| {
                        if ui.button("名前を変える").clicked() {
                            self.renaming = Some((index, self.decals[index].layer.name.clone().unwrap_or_default()));
                            ui.close();
                        }
                        ui.separator();
                        self.decal_menu(ui);
                    });
                }
                self.base_layer(ui, back as usize);
            }
            self.drag_layer(ui, &rows_at);
        });
        ui.add_space(6.0);
        self.psd_loader(ui);
    }

    /// レイヤーの行のドラッグ。運んでいる間は、落とす場所に線を出し、放したら、同じ面の中で重なり順を変える。
    fn drag_layer(&mut self, ui: &mut egui::Ui, rows: &[(usize, egui::Rect)]) {
        let Some(dragged) = self.layer_drag.filter(|&i| i < self.decals.len()) else {
            self.layer_drag = None;
            return;
        };
        let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) else { return };
        let back = self.decals[dragged].back;
        // 同じ面の行のうち、カーソルにいちばん近い行の、上か下。
        let target = rows
            .iter()
            .filter(|(i, _)| self.decals[*i].back == back)
            .min_by(|a, b| (a.1.center().y - pointer.y).abs().total_cmp(&(b.1.center().y - pointer.y).abs()))
            .map(|&(i, rect)| (i, rect, pointer.y < rect.center().y));
        let Some((over, rect, above)) = target else { return };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let y = if above { rect.top() } else { rect.bottom() };
        ui.painter().hline(rect.x_range(), y, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill));
        if ui.input(|i| i.pointer.any_released()) {
            self.layer_drag = None;
            self.move_layer(dragged, over, above);
        }
    }

    /// 柄 `dragged` を、柄 `over` のすぐ上（`above`）か、すぐ下へ動かす（重なり順）。動かしたら、その柄を選ぶ。
    pub(super) fn move_layer(&mut self, dragged: usize, over: usize, above: bool) {
        // 一覧は上が重なりの上。上に落としたら、その柄のすぐ上（並びでは、すぐ後ろ）へ。
        let mut to = if above { over + 1 } else { over };
        if dragged < to {
            to -= 1;
        }
        if to == dragged || dragged >= self.decals.len() || to >= self.decals.len() {
            return;
        }
        self.checkpoint();
        let item = self.decals.remove(dragged);
        self.decals.insert(to, item);
        self.select_only(Some(to));
        self.grab = None;
        (self.art_dirty, self.back_dirty) = (true, true);
    }

    /// 展開図まるごとの絵（PSD）の読み込み。
    fn psd_loader(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("展開図の絵を読み込む").default_open(!self.psd_info.is_empty()).show(ui, |ui| {
            ui.small("「出力」で保存した型紙の PSD に絵を描いて、ウィンドウにドロップします。パスを打っても読めます。裏の型紙（名前に「_裏」が付くもの）に描いた絵は、紙の裏に敷きます。");
            let mut load_clicked = false;
            let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
            ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                load_clicked = ui.button("読み込む").clicked();
                ui.add(egui::TextEdit::singleline(&mut self.psd_path).hint_text("PSD のパス").desired_width(f32::INFINITY));
            });
            if load_clicked {
                let path = PathBuf::from(self.psd_path.trim().trim_matches('"'));
                self.open_psd(&path);
            }
            if !self.psd_info.is_empty() {
                ui.small(&self.psd_info);
            }
        });
    }

    /// 左（型）: 型の一覧。
    pub(super) fn shape_list(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("shape-list").resizable(false).exact_size(180.0).show(ui, |ui| {
            ui.add_space(4.0);
            ui.strong("型");
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for (sides, label) in SHAPES {
                    if ui.selectable_label(self.dims.sides == sides, label).clicked() && self.dims.sides != sides {
                        if self.decals.is_empty() {
                            self.set_shape(sides);
                        } else {
                            // 置いた柄が外れるので、先に確かめる。
                            self.pending_shape = Some(sides);
                        }
                    }
                }
            });
        });
    }

    /// 右（型）: 寸法、紙の厚み、型紙の検査。
    pub(super) fn shape_settings(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("shape-settings").resizable(false).exact_size(300.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(4.0);
                ui.strong("寸法（内寸）");
                let gable = self.dims.sides == 2;
                let pillow = self.dims.sides == PILLOW;
                let prism = self.dims.sides >= 3 && !pillow;
                let dims = &mut self.dims;
                egui::Grid::new("dims").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str| {
                        ui.label(label);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().slider_width = 90.0;
                            ui.add(egui::Slider::new(value, range.clone()).show_value(false));
                            number(ui, value, range, unit);
                        });
                        ui.end_row();
                    };
                    row(ui, if prism { "一辺" } else if pillow { "横の長さ" } else { "幅" }, &mut dims.w, 40.0..=300.0, "mm");
                    if !prism && !pillow {
                        row(ui, "奥行き", &mut dims.d, 20.0..=200.0, "mm");
                    }
                    row(ui, if gable { "胴の高さ" } else if pillow { "面の高さ" } else { "高さ" }, &mut dims.h, 40.0..=300.0, "mm");
                    if gable {
                        row(ui, "屋根の高さ", &mut dims.roof, 10.0..=150.0, "mm");
                    }
                    if pillow {
                        row(ui, "ふくらみ", &mut dims.bulge, 15.0..=90.0, "度");
                    }
                    row(ui, "紙の厚み", &mut dims.t, 0.3..=6.0, "mm");
                });
                let d = self.dims;
                if gable && d.d > d.w {
                    ui.small("奥行きは幅までに抑えています（側面の三角形が中央でぶつかるため）。");
                }
                if pillow {
                    let psi = d.bulge.clamp(15.0, 90.0).to_radians();
                    let deepest = d.h * (1.0 - (psi * 0.5).cos()) / psi;
                    ui.small(format!("いちばん厚いところ: 約 {:.0} mm", deepest * 2.0));
                }
                if !prism && !gable && !pillow {
                    ui.small(format!("外寸: {:.1} × {:.1} × {:.1} mm", d.w + 2.0 * d.t, d.d + 2.0 * d.t, d.h + 4.0 * d.t));
                }
                ui.small(format!("展開図: {:.0} × {:.0} mm", self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]));
                ui.add_space(6.0);
                ui.separator();

                ui.strong("型紙の検査");
                if self.problems.is_empty() {
                    ui.label("問題は見つかりませんでした。");
                }
                for (blocking, text) in &self.problems {
                    if *blocking {
                        ui.colored_label(egui::Color32::from_rgb(240, 120, 100), format!("問題: {text}"));
                    } else {
                        ui.colored_label(egui::Color32::from_rgb(230, 170, 90), format!("注意: {text}"));
                    }
                }
                ui.small("閉じていない辺、開け口と留め、のりしろの貼る相手、展開図の重なりを見ています。");
            });
        });
    }

    /// 左（出力）: 出力の種類。
    pub(super) fn output_list(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("output-list").resizable(false).exact_size(240.0).show(ui, |ui| {
            ui.add_space(4.0);
            ui.strong("出力");
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for (index, (label, ready)) in OUTPUTS.iter().enumerate() {
                    let response = ui.add_enabled(*ready, egui::Button::selectable(self.output_kind == index, *label)).on_disabled_hover_text("これから作ります");
                    if response.clicked() {
                        self.output_kind = index;
                    }
                }
            });
        });
    }

    /// 型を変える前の確認。置いた柄が外れることを知らせる。
    pub(super) fn confirm_shape(&mut self, ctx: &egui::Context) {
        let Some(sides) = self.pending_shape else { return };
        let name = SHAPES.iter().find(|(s, _)| *s == sides).map_or("別の型", |(_, name)| name);
        let count = self.decals.len();
        let response = egui::Modal::new(egui::Id::unique("confirm-shape")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.strong(format!("型を「{name}」に変えますか"));
            ui.add_space(4.0);
            ui.label(format!("型を変えると、面の並びが変わるので、置いた柄 {count} つが箱から外れます。"));
            ui.label("変えたあとでも、Ctrl+Z で、前の型と柄に戻せます。");
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let change = ui.button("型を変える").clicked();
                let cancel = ui.button("やめる").clicked();
                (change, cancel)
            })
            .inner
        });
        let (change, cancel) = response.inner;
        if change {
            self.pending_shape = None;
            self.set_shape(sides);
        } else if cancel || response.should_close() {
            self.pending_shape = None;
        }
    }

    /// 立体の表示の下に置く、動きの操作。開け閉め（買った人の動き）と、組み立て（展開図から箱まで）。
    /// 幅が足りないときは、2 行に分ける。
    /// 開け閉めと組み立ての操作を、何行に分けるか。底も開く型では、操作が 1 つ多い。
    pub(super) fn motion_rows(&self, width: f32) -> usize {
        if self.template.can_open_at(1) {
            if width >= 900.0 {
                1
            } else if width >= 520.0 {
                2
            } else {
                3
            }
        } else if width >= 660.0 {
            1
        } else {
            2
        }
    }

    pub(super) fn motion_controls(&mut self, ui: &mut egui::Ui, rows: usize) {
        let width = ui.available_width();
        let bottom = self.template.can_open_at(1);
        let gap = |ui: &mut egui::Ui| {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
        };
        // 開け閉めを左に、大きく置く。デザイン中にいちばん動かすため。
        match (rows, bottom) {
            (1, false) => {
                let spare = (width - 420.0).max(120.0);
                ui.horizontal(|ui| {
                    self.open_controls(ui, 0, (spare * 0.62).clamp(60.0, 320.0));
                    gap(ui);
                    self.fold_controls(ui, (spare * 0.38).clamp(50.0, 200.0));
                });
            }
            (1, true) => {
                let spare = (width - 640.0).max(150.0);
                ui.horizontal(|ui| {
                    self.open_controls(ui, 0, (spare * 0.44).clamp(50.0, 280.0));
                    gap(ui);
                    self.open_controls(ui, 1, (spare * 0.26).clamp(40.0, 180.0));
                    gap(ui);
                    self.fold_controls(ui, (spare * 0.30).clamp(40.0, 200.0));
                });
            }
            (2, true) => {
                let spare = (width - 400.0).max(100.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        self.open_controls(ui, 0, (spare * 0.6).clamp(40.0, 280.0));
                        gap(ui);
                        self.open_controls(ui, 1, (spare * 0.4).clamp(40.0, 180.0));
                    });
                    ui.horizontal(|ui| self.fold_controls(ui, (width - 190.0).clamp(60.0, 320.0)));
                });
            }
            _ => {
                let slider = (width - 190.0).clamp(60.0, 320.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| self.open_controls(ui, 0, slider));
                    if bottom {
                        ui.horizontal(|ui| self.open_controls(ui, 1, slider));
                    }
                    ui.horizontal(|ui| self.fold_controls(ui, slider));
                });
            }
        }
    }

    /// 開け閉めの操作。`which` は、0 が主な開け口（上のフタ、屋根。ピロー型では左の口）、1 が底（ピロー型では右の口）。
    fn open_controls(&mut self, ui: &mut egui::Ui, which: usize, slider: f32) {
        let can_open = self.template.can_open_at(which);
        ui.add_enabled_ui(can_open, |ui| {
            // ピロー型には上と底の区別がないので、左右の口と呼ぶ。
            let pillow = self.dims.sides == super::PILLOW;
            ui.strong(match (which, pillow) {
                (0, false) => "開け閉め",
                (_, false) => "底",
                (0, true) => "左の口",
                (_, true) => "右の口",
            });
            if ui.button("閉じる").clicked() {
                self.fold = 1.0;
                self.open_to[which] = Some(0.0);
            }
            ui.spacing_mut().slider_width = slider;
            if ui.add(egui::Slider::new(&mut self.open[which], 0.0..=1.0).show_value(false)).changed() {
                // 開け閉めは、組み上がった箱に対する動き。
                self.fold = 1.0;
                self.animate_to = None;
                self.open_to[which] = None;
            }
            if ui.button("開ける").clicked() {
                self.fold = 1.0;
                self.open_to[which] = Some(1.0);
            }
        })
        .response
        .on_disabled_hover_text("この型には、開け閉めの動きがまだ入っていません");
    }

    fn fold_controls(&mut self, ui: &mut egui::Ui, slider: f32) {
        ui.strong("組み立て");
        if ui.button("展開図").clicked() {
            self.fold_to(0.0);
        }
        ui.spacing_mut().slider_width = slider;
        if ui.add(egui::Slider::new(&mut self.fold, 0.0..=1.0).show_value(false)).changed() {
            self.animate_to = None;
        }
        if ui.button("箱").clicked() {
            self.fold_to(1.0);
        }
    }

    /// いちばん下: 知らせと、いまの状態。
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        // 知らせは、出してから 8 秒で消す。
        let now = ui.input(|i| i.time);
        if let Some((_, since)) = &mut self.message {
            let started = *since.get_or_insert(now);
            if now - started > 8.0 {
                self.message = None;
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            }
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                if let Some((text, _)) = &self.message {
                    ui.small(text);
                    ui.separator();
                }
                // カーソルの下の面の名前と、いまの型。
                if let Some(panel) = self.hover.and_then(|i| self.template.panels.get(i)) {
                    ui.small(format!("カーソルの下: {}", panel.name));
                    ui.separator();
                }
                ui.label(egui::RichText::new(self.shape_summary()).small().weak());
                if self.debug {
                    ui.separator();
                    ui.label(egui::RichText::new(format!("パネル {} 枚　合わせ目 {} か所　三角形 {} 個　描画: {}", self.template.panels.len(), self.surface.seams.len(), self.triangles, self.backend)).small().weak());
                }
            });
        });
    }
}
