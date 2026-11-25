//! 型の工程の「ECMA の型」: 番号（上と底の閉じ方、フタの付く面）を選んで、内寸から型を作る。
//!
//! 形は `pakenoba_core::ecma` が描き起こす。番号は ECMA の呼び名で、メーカーとそのまま話が通じる。

use eframe::egui;
use pakenoba_core::ecma::{Closure, Code};
use pakenoba_i18n::t;

use super::{App, Dims, ECMA};

/// フタの付く面の名前。
fn side_name(front: bool) -> &'static str {
    if front { t!("Front panel") } else { t!("Back panel") }
}

/// 番号の短い説明（「上: 差し込みのフタ（正面）／底: 1-2-3 底（背面）」）。
pub(super) fn describe(code: Code) -> String {
    let part = |c: Closure, front: bool| if c.has_side() { format!("{}（{}）", c.label(), side_name(front)) } else { c.label().to_owned() };
    t!("Top: {} / Bottom: {}", part(code.top, code.top_front), part(code.bottom, code.bottom_front))
}

impl App {
    /// 型の一覧の「ECMA の型」を押したとき。
    pub(super) fn choose_ecma(&mut self) {
        let dims = Dims::for_shape(ECMA, self.dims.t);
        self.pick_ecma_dims(dims);
    }

    /// 番号を変える（パネルの並びが変わるので、置いた柄があれば先に確かめる）。
    pub(super) fn set_ecma(&mut self, code: Code) {
        let n = code.to_u32();
        if self.dims.sides == ECMA && self.dims.ecma == n {
            return;
        }
        let dims = if self.dims.sides == ECMA { Dims { ecma: n, ..self.dims } } else { Dims { ecma: n, ..Dims::for_shape(ECMA, self.dims.t) } };
        self.pick_ecma_dims(dims);
    }

    fn pick_ecma_dims(&mut self, dims: Dims) {
        if self.decals.is_empty() {
            self.apply_dims(dims, self.material);
        } else {
            self.pending_shape = Some(ECMA);
            self.pending_dims = Some((dims, self.material));
        }
    }

    /// 右（型）の「ECMA の番号」の欄。
    pub(super) fn ecma_settings(&mut self, ui: &mut egui::Ui) {
        let Some(code) = Code::from_u32(self.dims.ecma) else {
            ui.weak(t!("The ECMA number is not valid."));
            return;
        };
        ui.label(egui::RichText::new(code.name()).size(16.0).strong());
        ui.add(egui::Label::new(egui::RichText::new(describe(code)).weak()).wrap());
        ui.add_space(2.0);
        // 組んだ形と展開図を見比べて選ぶ一覧。
        if ui.add(egui::Button::new(t!("Choose from the pictures…")).min_size(egui::vec2(190.0, 26.0))).clicked() {
            self.open_ecma_gallery();
        }
        if !code.listed() {
            ui.small(t!("This combination is not in the public list of ECMA styles, but it can still be made."));
        }
        if code.simplified() {
            ui.small(t!("The bottom is drawn in a simplified form (the interlocking cuts and the glued corners are not drawn yet)."));
        }
        ui.add_space(4.0);
        let mut next = code;
        egui::Grid::new("ecma-parts").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            for (top, label) in [(true, t!("Top of the box")), (false, t!("Bottom of the box"))] {
                ui.label(egui::RichText::new(label).color(super::theme::MUTED));
                ui.vertical(|ui| {
                    let (closure, front) = if top { (&mut next.top, &mut next.top_front) } else { (&mut next.bottom, &mut next.bottom_front) };
                    egui::ComboBox::from_id_salt(("ecma-closure", top)).width(190.0).selected_text(format!("{:02}  {}", closure.number(), closure.label())).show_ui(ui, |ui| {
                        for c in Closure::ALL.into_iter().filter(|c| if top { c.on_top() } else { c.on_bottom() }) {
                            ui.selectable_value(closure, c, format!("{:02}  {}", c.number(), c.label()));
                        }
                    });
                    if closure.has_side() {
                        ui.horizontal(|ui| {
                            ui.weak(match closure {
                                Closure::Hanger => t!("Hanger on"),
                                Closure::Flaps | Closure::Reduced => t!("Outer flap on"),
                                Closure::Snap => t!("Locking flap on"),
                                _ => t!("Lid on"),
                            });
                            ui.selectable_value(front, false, side_name(false));
                            ui.selectable_value(front, true, side_name(true));
                        });
                    } else {
                        *front = false;
                    }
                });
                ui.end_row();
            }
        });
        // 番号を打って選ぶ。
        let id = egui::Id::unique("ecma-typed");
        let mut typed = ui.ctx().data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
        ui.horizontal(|ui| {
            let edit = ui.add(egui::TextEdit::singleline(&mut typed).desired_width(130.0).hint_text("A20.20.03.01"));
            let go = ui.add_enabled(!typed.trim().is_empty(), egui::Button::new(t!("Use this number"))).clicked() || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go {
                match Code::parse(&typed) {
                    Some(c) => {
                        next = c;
                        typed.clear();
                    }
                    None => self.say(t!("\"{}\" is not an ECMA number that PAKENOBA can make (class A: A20.20.03.01 and so on).", typed.trim())),
                }
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, typed));
        if next != code {
            self.set_ecma(next);
        }
    }
}
