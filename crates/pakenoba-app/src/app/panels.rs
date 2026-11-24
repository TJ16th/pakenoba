//! 画面の部品。上のタブ、上の帯、左の道具と一覧、右の設定、下の帯。
//!
//! 役割は工程をまたいで変えない。左は「選ぶもの・道具」、右は「選んだものの設定」、下は「動き」。

use pakenoba_i18n::t;

use eframe::egui;

use super::{App, SHAPES, Tool, View, Workspace, icons};
use pakenoba_core::template::TuckFlaps;


/// 出力の種類。（名前、もう使えるか）。
use super::outputs::OUTPUTS;

/// 数値の欄。右にそろえ、単位を必ず添える。
/// 押すたびに 2 つの動きを切り替えるボタン。幅は、長いほうの文字に合わせて変えない（押しても並びが動かない）。
fn toggle(ui: &mut egui::Ui, label: &str, both: [&str; 2]) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let widest = both.iter().map(|text| ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), egui::Color32::WHITE).size().x)).fold(0.0, f32::max);
    let width = widest + ui.spacing().button_padding.x * 2.0;
    ui.add_sized([width, ui.spacing().interact_size.y], egui::Button::new(label))
}

fn number(ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str) -> egui::Response {
    ui.add_sized([96.0, ui.spacing().interact_size.y], egui::DragValue::new(value).range(range).speed(0.5).max_decimals(1).suffix(format!(" {unit}")))
}

/// 上の帯の説明の 1 行。右の端の表示の切り替えに重ならないよう、はみ出す分は「…」にする（全文はカーソルを乗せると出る）。
fn hint(ui: &mut egui::Ui, text: &str) {
    let width = (ui.available_width() - 230.0).max(60.0);
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add(egui::Label::new(egui::RichText::new(text).weak()).truncate());
    });
}

/// 見出し（タブの帯）つきの灰色の箱（PhotoCraft の欄と同じ形）。`fill` なら、下の端まで伸ばす。
pub(super) fn boxed<R>(ui: &mut egui::Ui, title: &str, fill: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    super::theme::panel_frame()
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if fill {
                ui.set_min_height(ui.available_height() - 8.0);
            }
            icons::panel_tabs(ui, &[title], 0);
            super::theme::panel_body().show(ui, add).inner
        })
        .inner
}

impl App {
    /// 上（メニューバーの下）は 2 段。
    /// 1 段目: ホームと、開いている案件のタブ（ブラウザや PhotoCraft の書類のタブと同じ）。
    /// 2 段目: 選んでいるタブの案件の工程（型、デザイン、出力）と、その案件の素材。左から順に進めば完成する。
    pub(super) fn top_bar(&mut self, ui: &mut egui::Ui) {
        let bar = egui::Frame::new().fill(super::theme::BACKGROUND).inner_margin(egui::Margin { left: 4, right: 4, top: 3, bottom: 0 });
        egui::Panel::top("documents").frame(bar).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // はじめの画面（ホーム）。タブの並びの左の端。
                if icons::doc_tab(ui, t!("Home"), self.home, false, None).0.clicked() {
                    self.home = true;
                    self.library.open = false;
                }
                ui.add_space(6.0);
                self.tab_strip(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.is_saving() {
                        ui.weak(t!("Saving…"));
                    }
                });
            });
        });
        if self.home {
            return;
        }
        let steps = egui::Frame::new().fill(super::theme::PANEL).inner_margin(egui::Margin { left: 6, right: 6, top: 3, bottom: 3 });
        egui::Panel::top("workspaces").frame(steps).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (workspace, label) in [(Workspace::Shape, t!("Structure")), (Workspace::Design, t!("Design")), (Workspace::Output, t!("Output"))] {
                    // 「素材」を開いているあいだは、どの工程も選んでいない見た目にする。
                    if ui.selectable_label(!self.library.open && self.workspace == workspace, format!("  {label}  ")).clicked() {
                        self.go_to(workspace);
                    }
                }
                // 素材は流れの一歩ではないので、細い区切りを入れて、工程の右に置く。
                ui.separator();
                if ui.selectable_label(self.library.open, t!("  Assets  ")).on_hover_text(t!("The assets of this project, and folders and samples to import from (press again to go back)")).clicked() {
                    self.toggle_library();
                }
                // 元に戻すとやり直すは、「編集」のメニューと Ctrl+Z に移した。
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.selectable_label(self.agent.open, t!("  Agent  ")).on_hover_text(t!("Ask an AI to make changes (Anthropic or OpenAI)")).clicked() {
                        self.agent.open = !self.agent.open;
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
                            ui.label(t!("{} pieces of artwork", self.selection().len()));
                            ui.separator();
                            ui.label(t!("Align"));
                            self.align_buttons(ui);
                            ui.separator();
                            if ui.button(t!("Delete")).on_hover_text("Delete").clicked() {
                                self.cmd_delete();
                            }
                        }
                        Some(index) => {
                            ui.label(self.layer_label(index));
                            ui.separator();
                            ui.label(t!("Arrange"));
                            if ui.add_enabled(self.neighbor(index, 1).is_some(), egui::Button::new(t!("Bring forward"))).clicked() {
                                self.cmd_reorder(1);
                            }
                            if ui.add_enabled(self.neighbor(index, -1).is_some(), egui::Button::new(t!("Send backward"))).clicked() {
                                self.cmd_reorder(-1);
                            }
                            ui.separator();
                            if ui.add_enabled(!self.decals[index].layer.fixed(), egui::Button::new(t!("Rotate 90°"))).on_disabled_hover_text(t!("Locked")).clicked() {
                                self.rotate_selected();
                            }
                            ui.separator();
                            ui.label(t!("To the panel"));
                            self.align_buttons(ui);
                        }
                        None => {
                            hint(ui, t!("Click artwork to select it. Hold Shift to select several (Ctrl+A for all). Place assets by dragging them from the assets panel on the left onto the box or the dieline"));
                        }
                    },
                    Tool::Hand => {
                        hint(ui, t!("Drag to turn the box or move the dieline"));
                    }
                    Tool::Text => self.text_bar(ui),
                    Tool::Shape => self.shape_bar(ui),
                    Tool::Ai => {}
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    // 右から並べるので、逆の順に足す。
                    for (view, label, key) in [(View::Back, t!("Back"), "3"), (View::Front, t!("Front"), "2"), (View::Solid, t!("3D"), "1")] {
                        let response = ui.selectable_label(self.shown.get(view), format!(" {label} ")).on_hover_text(t!("Show or hide the {label} view. Press {key}, or click with Alt held, to show only this view"));
                        if response.clicked() {
                            if ui.input(|i| i.modifiers.alt) {
                                self.shown.solo(view);
                            } else if !self.shown.toggle(view) {
                                self.say(t!("The last view cannot be hidden."));
                            }
                        }
                    }
                    ui.weak(t!("View"));
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
        // PhotoCraft と同じく、欄ごとに明るめの灰色の箱にし、箱の間と外を暗い溝にする。
        let side = egui::Frame::new().fill(super::theme::BACKGROUND).inner_margin(egui::Margin::same(4));
        egui::Panel::right("design-side").resizable(false).exact_size(314.0).frame(side).show(ui, |ui| {
            // レイヤーの欄は下に固定する（上の欄の高さが、選んだ柄で変わっても、レイヤーの欄は動かない）。
            // 境目は上下に動かせる（高さは覚えておく）。
            let split_id = egui::Id::unique("design-layers-height");
            let total = ui.available_height();
            let stored = ui.ctx().data_mut(|d| *d.get_persisted_mut_or(split_id, 380.0f32));
            let layers_h = stored.clamp(180.0, (total - 120.0).max(180.0));
            let width = ui.available_width();
            let top_h = (total - layers_h - 6.0).max(40.0);
            egui::ScrollArea::vertical().id_salt("properties").max_height(top_h).min_scrolled_height(top_h).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                super::theme::panel_frame().show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    icons::panel_tabs(ui, &[t!("Properties")], 0);
                    super::theme::panel_body().show(ui, |ui| {
                        // 型の工程へ戻る入口。言語によっては長くなるので、折り返す。
                        ui.horizontal_wrapped(|ui| {
                            if ui.small_button(t!("Structure")).on_hover_text(t!("Go back to the Structure step to change the box type and dimensions")).clicked() {
                                self.workspace = Workspace::Shape;
                            }
                            ui.weak(self.shape_summary());
                        });
                        self.properties(ui);
                    });
                });
            });
            // 境目のつまみ（上下に引くと、レイヤーの欄の高さが変わる）。
            let (handle, response) = ui.allocate_exact_size(egui::vec2(width, 6.0), egui::Sense::drag());
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                ui.painter().hline(handle.x_range(), handle.center().y, egui::Stroke::new(2.0, ui.visuals().selection.bg_fill));
            }
            if response.dragged() {
                let next = (layers_h - response.drag_delta().y).clamp(180.0, (total - 120.0).max(180.0));
                ui.ctx().data_mut(|d| d.insert_persisted(split_id, next));
            }
            ui.allocate_ui(egui::vec2(width, layers_h), |ui| {
                super::theme::panel_frame().show(ui, |ui| {
                    ui.set_min_size(egui::vec2(ui.available_width(), layers_h - 8.0));
                    ui.set_max_height(layers_h - 8.0);
                    ui.spacing_mut().item_spacing.y = 6.0;
                    self.layers(ui);
                });
            });
        });
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        if self.selected.is_none()
            && let Some(index) = self.finish_selected.filter(|&i| i < self.finishes.len())
        {
            return self.finish_properties(ui, index);
        }
        let Some(index) = self.selected.filter(|&i| i < self.decals.len()) else {
            ui.weak(t!("Select artwork to see its settings here."));
            return;
        };
        if !crate::paint::is_artwork(&self.decals[index].picture) {
            return self.layer_properties(ui, index);
        }
        // 見出し: レイヤーの名前と、右に種類（PhotoCraft の「Layer 1 … Pixel Layer」と同じ）。
        let title = self.layer_label(index);
        let kind = self.decals[index].picture.label();
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
            icons::draw_image(ui.painter(), rect.center(), egui::Stroke::new(1.1, super::theme::TEXT));
            ui.label(egui::RichText::new(title).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.weak(kind));
        });
        let others = self.selection().len() - 1;
        if others > 0 {
            ui.weak(t!("{others} more selected. They move together."));
        }
        let locked = self.decals[index].layer.locked;
        let fixed = self.decals[index].layer.fixed();
        if locked {
            ui.weak(t!("Locked. To move it, unlock it in the Layers panel."));
        } else if fixed {
            ui.weak(t!("Position is locked. Unlock it in the Layers panel to move it."));
        } else if self.decals[index].layer.lock_pixels {
            ui.weak(t!("Image is locked. Unlock it in the Layers panel to change adjustments or the mask."));
        }
        self.brush_options(ui);
        let earlier = self.snapshot();
        let template = &self.template;
        let ratio_lock = &mut self.ratio_lock;
        let decal = &mut self.decals[index];
        let before = decal.decal;
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
        // 変形（PhotoCraft の「Transform」と同じ並び: W と H の間に鎖、その下に X と Y）。
        // 欄の幅から、数値の欄の幅を決める（はみ出さないように。見出しの字下げのぶんも引く）。
        let fw = ((ui.available_width() - 24.0 - 84.0) / 2.0).clamp(56.0, 90.0);
        egui::CollapsingHeader::new(egui::RichText::new(t!("Transform")).strong()).id_salt("transform").default_open(true).show(ui, |ui| {
            let label = |ui: &mut egui::Ui, text: &str| {
                ui.add_sized([22.0, 22.0], egui::Label::new(egui::RichText::new(text).color(super::theme::MUTED)));
            };
            let field = |ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str| {
                ui.add_sized([fw, 22.0], egui::DragValue::new(value).range(range).speed(0.5).max_decimals(1).suffix(unit));
            };
            ui.add_enabled_ui(!fixed, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    label(ui, t!("W"));
                    field(ui, &mut width, 2.0..=2000.0, " mm");
                    if icons::link_button(ui, *ratio_lock).clicked() {
                        *ratio_lock = !*ratio_lock;
                    }
                    label(ui, t!("H"));
                    field(ui, &mut height, 2.0..=2000.0, " mm");
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    label(ui, t!("X"));
                    field(ui, &mut x, -2000.0..=4000.0, " mm");
                    ui.add_space(22.0);
                    label(ui, t!("Y"));
                    field(ui, &mut y, -2000.0..=4000.0, " mm");
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    label(ui, t!("Angle"));
                    field(ui, &mut degrees, -180.0..=180.0, t!("°"));
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(t!("Side of sheet")).color(super::theme::MUTED));
                    ui.selectable_value(&mut back, false, t!(" Front ")).on_hover_text(t!("Place on the front of the sheet (outside of the box)"));
                    ui.selectable_value(&mut back, true, t!(" Back ")).on_hover_text(t!("Place on the back of the sheet (inside of the box), such as under the lid"));
                });
            });
        });
        if (x, y) != (x0, y0) {
            // 数値で動かすときは、置かれている面の中にとどめる（隣の面へは、ドラッグで動かす）。
            decal.set_center(template, [template.min[0] + mirror(x), template.max[1] - y]);
            decal.at = [decal.at[0].clamp(0.0, 1.0), decal.at[1].clamp(0.0, 1.0)];
        }
        if !*ratio_lock {
            decal.size = [width, height];
        } else if width != decal.size[0] {
            // 鎖をつないでいるあいだは、縦横の比を保つ。
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
            self.log_transform();
        }
        if moved_side {
            self.back_dirty = true;
            self.grab = None;
            self.say(if back { t!("Moved to the back of the sheet. You can see it in the \"Back\" view or inside the opened box.") } else { t!("Moved to the front of the sheet.") });
        }
        // どの面にかかっているか。
        let pieces = self.surface.layout(&self.template, &self.decals[index]);
        let mut names: Vec<&str> = pieces.iter().map(|p| pakenoba_i18n::trc("panel", self.template.panels[p.panel].name)).collect();
        names.dedup();
        let seams = pieces.iter().map(|p| p.seams).max().unwrap_or(0);
        ui.add_space(2.0);
        ui.small(t!("On panels: {}", names.join(pakenoba_i18n::tr(", "))));
        if seams > 0 {
            ui.small(t!("Crosses {seams} seam(s)."));
        }
        // 中身ごとの設定（文字、図形）、柄だけの調整、画像の道具、外のアプリで直す。
        self.text_properties(ui, index, locked);
        self.shape_properties(ui, index, locked);
        self.adjust_properties(ui, index, locked);
        self.adjust_controls(ui, index, locked);
        self.edit_controls(ui, index);
        ui.add_space(4.0);
        if ui.add_enabled(!locked, egui::Button::new(t!("Delete this artwork"))).on_hover_text("Delete").on_disabled_hover_text(t!("Locked artwork cannot be deleted")).clicked() {
            self.cmd_delete();
        }
    }

    /// 左（型）: 型の一覧。
    pub(super) fn shape_list(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("shape-list").resizable(false).exact_size(180.0).frame(super::theme::side_frame()).show(ui, |ui| {
            boxed(ui, t!("Box type"), true, |ui| ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for (sides, label) in SHAPES {
                    if ui.selectable_label(self.dims.sides == sides, pakenoba_i18n::tr(label)).clicked() && self.dims.sides != sides {
                        if self.decals.is_empty() {
                            self.set_shape(sides);
                        } else {
                            // 置いた柄が外れるので、先に確かめる。
                            self.pending_shape = Some(sides);
                        }
                    }
                }
            }));
        });
    }

    /// 右（型）: 寸法、紙の厚み、型紙の検査。
    pub(super) fn shape_settings(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("shape-settings").resizable(false).exact_size(320.0).frame(super::theme::side_frame()).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let gable = self.dims.sides == 2;
                let pillow = super::is_pillow(self.dims.sides);
                let prism = self.dims.sides >= 3 && !pillow;
                boxed(ui, t!("Dimensions (inside)"), false, |ui| {
                    let dims = &mut self.dims;
                    egui::Grid::new("dims").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        let row = |ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, unit: &str| {
                            ui.label(egui::RichText::new(label).color(super::theme::MUTED));
                            ui.horizontal(|ui| {
                                ui.spacing_mut().slider_width = 90.0;
                                ui.add(egui::Slider::new(value, range.clone()).show_value(false));
                                number(ui, value, range, unit);
                            });
                            ui.end_row();
                        };
                        row(ui, if prism { t!("Side") } else if pillow { t!("Horizontal length") } else { t!("Width") }, &mut dims.w, 40.0..=300.0, "mm");
                        if !prism && !pillow {
                            row(ui, t!("Depth"), &mut dims.d, 20.0..=200.0, "mm");
                        }
                        row(ui, if gable { t!("Body height") } else if pillow { t!("Face height") } else { t!("Height") }, &mut dims.h, 40.0..=300.0, "mm");
                        if gable {
                            row(ui, t!("Roof height"), &mut dims.roof, 10.0..=150.0, "mm");
                        }
                        if pillow {
                            row(ui, t!("Bulge"), &mut dims.bulge, 15.0..=90.0, t!("°"));
                        }
                        row(ui, t!("Paper thickness"), &mut dims.t, 0.3..=6.0, "mm");
                    });
                    let d = self.dims;
                    ui.add_space(4.0);
                    if gable && d.d > d.w {
                        ui.small(t!("The depth is limited to the width (the side triangles would collide in the middle)."));
                    }
                    if pillow {
                        let psi = d.bulge.clamp(15.0, 90.0).to_radians();
                        let deepest = d.h * (1.0 - (psi * 0.5).cos()) / psi;
                        ui.small(t!("Thickest point: about {:.0} mm", deepest * 2.0));
                    }
                    if !prism && !gable && !pillow {
                        ui.small(t!("Outside: {:.1} × {:.1} × {:.1} mm", d.w + 2.0 * d.t, d.d + 2.0 * d.t, d.h + 4.0 * d.t));
                    }
                    ui.small(t!("Dieline: {:.0} × {:.0} mm", self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]));
                });
                if self.dims.sides == 0 {
                    boxed(ui, t!("Flaps"), false, |ui| self.flap_settings(ui));
                }
                boxed(ui, t!("Paper and coating"), false, |ui| self.material_settings(ui));
                boxed(ui, t!("Dieline check"), false, |ui| {
                    if self.problems.is_empty() {
                        ui.label(t!("No problems were found."));
                    }
                    for (blocking, text) in &self.problems {
                        if *blocking {
                            ui.colored_label(super::theme::BAD, t!("Problem: {text}"));
                        } else {
                            ui.colored_label(super::theme::WARN, t!("Note: {text}"));
                        }
                    }
                    ui.small(t!("Checks for open edges, openings and their closures, what each glue flap sticks to, and overlaps in the dieline."));
                });
            });
        });
    }

    /// 差し込み箱のフラップの長さ。寸法と同じく、つまみと数の欄で変える。
    /// 決めていないあいだは寸法から決めた長さ（自動）を出し、右に薄く「自動」と出す。
    /// つまみか数を動かすと、その長さに決まる。右の「自動」を押すと、自動に戻る。
    fn flap_settings(&mut self, ui: &mut egui::Ui) {
        let d = &mut self.dims;
        let auto = TuckFlaps::auto(d.w, d.d, d.h, d.t);
        let ranges = TuckFlaps::ranges(d.w, d.d, d.h, d.t);
        let labels = [t!("Glue flap width"), t!("Side flap length"), t!("Tuck length")];
        let TuckFlaps { glue, dust, tuck } = &mut d.flaps;
        egui::Grid::new("flaps").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            for (i, chosen) in [glue, dust, tuck].into_iter().enumerate() {
                let range = ranges[i].clone();
                let mut value = chosen.unwrap_or(auto[i]).clamp(*range.start(), *range.end());
                ui.label(egui::RichText::new(labels[i]).color(super::theme::MUTED));
                ui.horizontal(|ui| {
                    // 英語の名前でも欄に収まるよう、寸法の行より少し詰める。
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.spacing_mut().slider_width = 64.0;
                    let slid = ui.add(egui::Slider::new(&mut value, range.clone()).show_value(false)).changed();
                    let typed = ui.add_sized([80.0, ui.spacing().interact_size.y], egui::DragValue::new(&mut value).range(range.clone()).speed(0.5).max_decimals(1).suffix(" mm")).changed();
                    if slid || typed {
                        *chosen = Some(value);
                    }
                    if chosen.is_some() {
                        if ui.small_button(t!("Auto")).on_hover_text(t!("Back to the length set from the dimensions")).clicked() {
                            *chosen = None;
                        }
                    } else {
                        ui.weak(t!("Auto")).on_hover_text(t!("Set from the dimensions. Move the slider to choose your own length."));
                    }
                });
                ui.end_row();
            }
        });
        ui.small(t!("The tuck and side flaps are the same at the top and bottom."));
    }

    /// 左（出力）: 出力の種類。
    pub(super) fn output_list(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("output-list").resizable(false).exact_size(240.0).frame(super::theme::side_frame()).show(ui, |ui| {
            boxed(ui, t!("Output"), true, |ui| ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for (index, (label, ready)) in OUTPUTS.iter().enumerate() {
                    let response = ui.add_enabled(*ready, egui::Button::selectable(self.output_kind == index, pakenoba_i18n::tr(label))).on_disabled_hover_text(t!("Coming later"));
                    if response.clicked() {
                        self.output_kind = index;
                    }
                }
            }));
        });
    }

    /// 型を変える前の確認。置いた柄が外れることを知らせる。
    pub(super) fn confirm_shape(&mut self, ctx: &egui::Context) {
        let Some(sides) = self.pending_shape else { return };
        let name = SHAPES.iter().find(|(s, _)| *s == sides).map_or(t!("another box type"), |(_, name)| pakenoba_i18n::tr(name));
        let count = self.decals.len();
        let buttons = [super::dialog::Button::accept(t!("Change box type"), true), super::dialog::Button::cancel(t!("Cancel"))];
        let picked = super::dialog::show(ctx, "confirm-shape", &t!("Change the box type to \"{name}\"?"), 400.0, &buttons, |ui| {
            ui.label(t!("Changing the box type changes the panels, so {count} placed piece(s) of artwork will come off the box."));
            ui.label(t!("Even after changing, Ctrl+Z gets the previous box type and artwork back."));
        });
        match picked {
            Some(0) => {
                self.pending_shape = None;
                self.set_shape(sides);
            }
            Some(_) => self.pending_shape = None,
            None => {}
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
            let pillow = super::is_pillow(self.dims.sides);
            ui.strong(match (which, pillow) {
                (0, false) => t!("Open / close"),
                (_, false) => t!("Base"),
                (0, true) => t!("Left end"),
                (_, true) => t!("Right end"),
            });
            // 1 つのボタンで、開けると閉じるを切り替える（動いている途中で押せば、折り返す）。
            // 文字は、押したときにすること。
            let heading = self.open_to[which].unwrap_or(self.open[which]);
            let opened = heading >= 0.5;
            if toggle(ui, if opened { t!("Close") } else { t!("Open up") }, [t!("Close"), t!("Open up")]).clicked() {
                self.fold = 1.0;
                self.open_to[which] = Some(if opened { 0.0 } else { 1.0 });
            }
            ui.spacing_mut().slider_width = slider;
            if ui.add(egui::Slider::new(&mut self.open[which], 0.0..=1.0).show_value(false)).changed() {
                // 開け閉めは、組み上がった箱に対する動き。
                self.fold = 1.0;
                self.animate_to = None;
                self.open_to[which] = None;
            }
        })
        .response
        .on_disabled_hover_text(t!("This box type does not have an opening motion yet"));
    }

    fn fold_controls(&mut self, ui: &mut egui::Ui, slider: f32) {
        ui.strong(t!("Assembly"));
        // 開け閉めと同じく、1 つのボタンで、展開図と箱を切り替える。
        let boxed = self.animate_to.unwrap_or(self.fold) >= 0.5;
        if toggle(ui, if boxed { t!("Flat") } else { t!("Box") }, [t!("Flat"), t!("Box")]).clicked() {
            self.fold_to(if boxed { 0.0 } else { 1.0 });
        }
        ui.spacing_mut().slider_width = slider;
        if ui.add(egui::Slider::new(&mut self.fold, 0.0..=1.0).show_value(false)).changed() {
            self.animate_to = None;
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
                    ui.small(t!("Under the cursor: {}", pakenoba_i18n::trc("panel", panel.name)));
                    ui.separator();
                }
                ui.label(egui::RichText::new(self.shape_summary()).small().weak());
                if self.debug {
                    ui.separator();
                    ui.label(egui::RichText::new(t!("{} panels  {} seams  {} triangles  renderer: {}", self.template.panels.len(), self.surface.seams.len(), self.triangles, self.backend)).small().weak());
                }
            });
        });
    }
}
