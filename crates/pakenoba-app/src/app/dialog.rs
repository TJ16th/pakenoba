//! 窓（ダイアログ）の枠。PhotoCraft の `Dialog`（craft-libs の ui-panels の `dialog/modal.rs` と `sections/footer.rs`）に合わせる。
//!
//! - いちばん上は題の帯。題と、右の端に閉じる ×。下に区切りの線。
//! - 中身は、まわりに余白をとって置く。
//! - いちばん下に、右にそろえたボタン。主なボタン（OK、保存など）は強調の青で塗る。
//!   並びは OS に合わせる（Windows は主なボタンが先で、取り消しがあと。macOS と Linux は取り消しが先で、主なボタンがいちばん右）。
//! - Enter で主なボタン（押せるときだけ）、Esc で取り消し。開いている間は、うしろの画面を触れない。

use eframe::egui;

use super::theme;

/// ボタンの役目。見た目と、並ぶ場所が決まる。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Role {
    /// 主なボタン（青で塗る。Enter でも押される）。
    Accept,
    /// 取り消し（Esc と × でも押される）。
    Cancel,
}

pub(super) struct Button<'a> {
    pub label: &'a str,
    pub role: Role,
    pub enabled: bool,
}

impl<'a> Button<'a> {
    pub fn accept(label: &'a str, enabled: bool) -> Self {
        Button { label, role: Role::Accept, enabled }
    }

    pub fn cancel(label: &'a str) -> Self {
        Button { label, role: Role::Cancel, enabled: true }
    }
}

/// ボタンを並べる順（Windows は主なボタンが先）。
fn order(buttons: &[Button], windows: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..buttons.len()).collect();
    order.sort_by_key(|&i| match (windows, buttons[i].role) {
        (true, Role::Accept) | (false, Role::Cancel) => 0,
        _ => 1,
    });
    order
}

/// 窓を出す。押されたボタン（Enter、Esc、× も含む）の番号を返す。
pub(super) fn show(ctx: &egui::Context, id: &str, title: &str, width: f32, buttons: &[Button], body: impl FnOnce(&mut egui::Ui)) -> Option<usize> {
    let mut picked = None;
    let frame = egui::Frame::new().fill(theme::PANEL).stroke(egui::Stroke::new(1.0, theme::CONTROL_EDGE)).corner_radius(6.0).inner_margin(0.0);
    egui::Modal::new(egui::Id::unique(id)).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 8.0;
        // 題の帯。
        let (band, _) = ui.allocate_exact_size(egui::vec2(width, 36.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.text(egui::pos2(band.left() + 16.0, band.center().y), egui::Align2::LEFT_CENTER, title, egui::TextStyle::Button.resolve(ui.style()), theme::TEXT);
        painter.hline(band.x_range(), band.bottom() - 0.5, egui::Stroke::new(1.0, theme::DIVIDER));
        let close = egui::Rect::from_center_size(egui::pos2(band.right() - 20.0, band.center().y), egui::vec2(22.0, 22.0));
        let close_response = ui.interact(close, egui::Id::unique(id).with("close"), egui::Sense::click());
        if close_response.hovered() {
            ui.painter().rect_filled(close, 3.0, theme::HOVER);
        }
        let c = close.center();
        let stroke = egui::Stroke::new(1.3, if close_response.hovered() { theme::TEXT } else { theme::MUTED });
        ui.painter().line_segment([c + egui::vec2(-4.5, -4.5), c + egui::vec2(4.5, 4.5)], stroke);
        ui.painter().line_segment([c + egui::vec2(-4.5, 4.5), c + egui::vec2(4.5, -4.5)], stroke);
        if close_response.clicked() {
            picked = buttons.iter().position(|b| b.role == Role::Cancel);
        }
        // 中身。
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 6, bottom: 4 }).show(ui, |ui| {
            ui.set_width(width - 32.0);
            body(ui);
        });
        // ボタン（右にそろえる）。
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 4, bottom: 14 }).show(ui, |ui| {
            ui.set_width(width - 32.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                // 右から置くので、並べる順の逆に置く。
                for &i in order(buttons, cfg!(windows)).iter().rev() {
                    let b = &buttons[i];
                    let text = egui::RichText::new(b.label).color(if b.role == Role::Accept && b.enabled { egui::Color32::WHITE } else { theme::TEXT });
                    let mut button = egui::Button::new(text).min_size(egui::vec2(84.0, 28.0)).corner_radius(4.0);
                    button = if b.role == Role::Accept { button.fill(theme::ACCENT) } else { button.fill(theme::PANEL).stroke(egui::Stroke::new(1.0, theme::OUTLINE)) };
                    if ui.add_enabled(b.enabled, button).clicked() {
                        picked = Some(i);
                    }
                }
            });
        });
    });
    // Enter で主なボタン、Esc で取り消し。
    let (enter, escape) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if picked.is_none() && enter {
        picked = buttons.iter().position(|b| b.role == Role::Accept && b.enabled);
    }
    if picked.is_none() && escape {
        picked = buttons.iter().position(|b| b.role == Role::Cancel);
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 主なボタンはwindowsでは先_ほかでは最後() {
        let buttons = [Button::cancel("Cancel"), Button::accept("OK", true)];
        assert_eq!(order(&buttons, true), [1, 0]);
        assert_eq!(order(&buttons, false), [0, 1]);
    }
}
