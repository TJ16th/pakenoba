//! 立体の表示の照らし方: 景色（環境マップ）を選ぶ、回す、明るさ。紙とコートの選び方。
//!
//! 景色の用意（スタジオは計算、屋外は取ってくる）は重いので、別のスレッドで作り、できたら GPU へ送る。

use pakenoba_i18n::t;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui;
use eframe::egui_wgpu;
use pakenoba_core::material::{Coat, Paper};

use super::App;
use crate::env::{self, Prepared};
use crate::gpu::{BoxRenderer, Look};

/// 景色の用意の進み具合。
#[derive(Default)]
pub(super) struct EnvState {
    /// いま GPU にある景色の名前。
    shown: Option<String>,
    /// 用意している景色の名前と、できたものの受け口。
    job: Option<(String, Receiver<Result<Prepared, String>>)>,
    /// 用意できなかった景色（同じものを何度も取りに行かない）。
    failed: Option<String>,
}

impl EnvState {
    /// いま用意している景色。
    pub fn loading(&self) -> Option<&str> {
        self.job.as_ref().map(|(key, _)| key.as_str())
    }
}

impl App {
    /// 描くときの、素材と照らし方。`yaw` は視点の左右の向き（景色は視点といっしょに回る）。
    pub(super) fn look(&self, yaw: f32) -> Look {
        let l = &self.settings.lighting;
        Look { material: self.material, turn: yaw + l.turn.to_radians(), exposure: 2f32.powf(l.exposure) }
    }

    /// 選んだ景色を用意し、できていたら GPU へ送る。描く前に毎回呼ぶ。
    pub(super) fn sync_environment(&mut self, ctx: &egui::Context, rs: &egui_wgpu::RenderState) {
        let wanted = env::preset(&self.settings.lighting.environment).key;
        if let Some((key, rx)) = &self.env.job {
            match rx.try_recv() {
                Ok(result) => {
                    let key = key.clone();
                    self.env.job = None;
                    match result {
                        Ok(prepared) => {
                            self.renderer.get_or_insert_with(|| BoxRenderer::new(rs)).set_environment(rs, &prepared);
                            self.env.shown = Some(key);
                            ctx.request_repaint();
                        }
                        Err(why) => {
                            self.env.failed = Some(key);
                            self.say(why);
                        }
                    }
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(TryRecvError::Disconnected) => self.env.job = None,
            }
        }
        let busy = self.env.job.is_some();
        if !busy && self.env.shown.as_deref() != Some(wanted) && self.env.failed.as_deref() != Some(wanted) {
            let (tx, rx) = std::sync::mpsc::channel();
            let preset = env::preset(wanted);
            std::thread::spawn(move || {
                let _ = tx.send(env::load(preset));
            });
            self.env.job = Some((wanted.to_owned(), rx));
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// 立体の表示の右上の「照明」の中身: 景色、回し、明るさ。
    pub(super) fn lighting_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(240.0);
        let mut changed = false;
        let mut save_now = false;
        ui.label(egui::RichText::new(t!("Environment")).strong());
        let current = env::preset(&self.settings.lighting.environment).key;
        for preset in env::PRESETS {
            let mut text = preset.label().to_owned();
            if self.env.loading() == Some(preset.key) {
                text = t!("{text} (preparing…)");
            } else if preset.downloads() && !preset.cached() {
                text = t!("{text} (downloads on first use)");
            }
            if ui.radio(current == preset.key, text).clicked() && current != preset.key {
                self.settings.lighting.environment = preset.key.to_owned();
                // 前に取ってこられなかったものも、選び直したら、もう一度試す。
                if self.env.failed.as_deref() == Some(preset.key) {
                    self.env.failed = None;
                }
                save_now = true;
            }
        }
        ui.small(t!("Outdoor environments are from Poly Haven (CC0)."));
        ui.separator();
        egui::Grid::new("lighting").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(t!("Turn"));
            let r = ui.add(egui::Slider::new(&mut self.settings.lighting.turn, -180.0..=180.0).suffix(t!("°")).fixed_decimals(0));
            changed |= r.changed();
            save_now |= r.drag_stopped() || (r.changed() && !r.dragged());
            ui.end_row();
            ui.label(t!("Brightness"));
            let r = ui.add(egui::Slider::new(&mut self.settings.lighting.exposure, -2.0..=2.0).step_by(0.1).fixed_decimals(1));
            changed |= r.changed();
            save_now |= r.drag_stopped() || (r.changed() && !r.dragged());
            ui.end_row();
        });
        if ui.button(t!("Reset turn and brightness")).clicked() {
            (self.settings.lighting.turn, self.settings.lighting.exposure) = (0.0, 0.0);
            save_now = true;
        }
        if changed || save_now {
            ui.ctx().request_repaint();
        }
        if save_now && let Err(e) = self.settings.save() {
            self.say(e);
        }
    }

    /// 紙とコートを選ぶ欄の中身（型の工程の右の欄）。
    pub(super) fn material_settings(&mut self, ui: &mut egui::Ui) {
        let before = self.material;
        egui::Grid::new("material").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(egui::RichText::new(t!("Paper")).color(super::theme::MUTED));
            egui::ComboBox::from_id_salt("paper").width(190.0).selected_text(self.material.paper.label()).show_ui(ui, |ui| {
                for paper in Paper::ALL {
                    ui.selectable_value(&mut self.material.paper, paper, paper.label());
                }
            });
            ui.end_row();
            ui.label(egui::RichText::new(t!("Coating")).color(super::theme::MUTED));
            egui::ComboBox::from_id_salt("coat").width(190.0).selected_text(self.material.coat.label()).show_ui(ui, |ui| {
                for coat in Coat::ALL {
                    ui.selectable_value(&mut self.material.coat, coat, coat.label());
                }
            });
            ui.end_row();
        });
        ui.small(t!("The coating is on the outside of the box only."));
        // 選び直したら、操作の記録に残す（コマンドを通す）。
        if self.material != before {
            let after = self.material;
            self.material = before;
            self.person("material.set", serde_json::json!({ "paper": after.paper.key(), "coat": after.coat.key() }));
        }
    }
}
