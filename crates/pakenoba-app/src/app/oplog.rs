//! 履歴の窓（表示 → 履歴）。「操作の記録」のタブに、新しい順に、いつ、誰が、何をしたかを並べる。「版」のタブは versions.rs。
//!
//! あとで、エージェントの欄の隣の「履歴」のタブに移す（docs/agent-and-mcp.md の 4 章）。

use pakenoba_i18n::t;
use eframe::egui;
use serde_json::Value;

use super::App;

/// コマンドの、画面に出す名前。
pub(super) fn command_label(name: &str) -> String {
    let label = match name {
        "decal.add" => t!("Added artwork"),
        "decal.delete" => t!("Deleted layers"),
        "decal.transform" => t!("Moved, resized or rotated"),
        "layer.select" => t!("Selected layers"),
        "layer.rename" => t!("Renamed a layer"),
        "layer.visible" => t!("Showed or hid layers"),
        "layer.lock" => t!("Locked or unlocked layers"),
        "layer.blend.set" => t!("Changed the blend mode"),
        "layer.opacity.set" => t!("Changed the opacity"),
        "layer.fill.set" => t!("Changed the fill"),
        "layer.reorder" => t!("Changed the stacking order"),
        "layer.clip" => t!("Changed the clipping mask"),
        "group.create" => t!("Grouped layers"),
        "group.ungroup" => t!("Ungrouped"),
        "adjust.add" => t!("Added an adjustment"),
        "adjust.set" => t!("Changed an adjustment"),
        "adjust.toggle" => t!("Turned an adjustment on or off"),
        "adjust.move" => t!("Reordered adjustments"),
        "adjust.remove" => t!("Removed an adjustment"),
        "adjustment_layer.add" => t!("Added an adjustment layer"),
        "mask.add" => t!("Added a mask"),
        "mask.invert" => t!("Inverted a mask"),
        "mask.enable" => t!("Turned a mask on or off"),
        "mask.delete" => t!("Deleted a mask"),
        "material.set" => t!("Changed the paper or coating"),
        "finish.add" => t!("Added a finish (foil or embossing)"),
        "finish.set" => t!("Changed a finish"),
        "finish.delete" => t!("Removed a finish"),
        "shape.set" => t!("Changed the box type"),
        "shape.dimensions" => t!("Changed the dimensions"),
        "asset.import" => t!("Imported an asset"),
        "text.set" => t!("Changed text"),
        "shape_art.set" => t!("Changed a shape"),
        "export.psd" => t!("Exported the dieline (PSD)"),
        "export.pdf" => t!("Exported the full-size dieline (PDF)"),
        "project.new" => t!("New project"),
        "project.open" => t!("Opened a project"),
        "project.save_as" => t!("Saved as"),
        "version.restore" => t!("Went back to a version"),
        "version.keep" => t!("Kept a version"),
        "edit.undo" => t!("Undo"),
        "edit.redo" => t!("Redo"),
        other => return other.to_owned(),
    };
    label.to_owned()
}

/// 誰が、を画面に出す形に。
pub(super) fn actor_label(actor: &str) -> String {
    match actor.split_once(':') {
        None if actor == "person" => t!("You").to_owned(),
        Some(("agent", name)) => t!("Agent ({name})"),
        Some(("mcp", name)) => t!("MCP ({name})"),
        _ => actor.to_owned(),
    }
}

/// 対象のレイヤー（印）を、短い文字に。
fn targets(args: &Value) -> String {
    let ids: Vec<String> = match (args.get("ids"), args.get("id")) {
        (Some(Value::Array(list)), _) => list.iter().map(|v| v.to_string()).collect(),
        (_, Some(id)) => vec![id.to_string()],
        _ => Vec::new(),
    };
    if ids.is_empty() { String::new() } else { t!("layer {}", ids.join(", ")) }
}

impl App {
    pub(super) fn log_window(&mut self, ctx: &egui::Context) {
        if !self.log_open {
            return;
        }
        let mut open = true;
        let mut record = self.settings.record_log;
        egui::Window::new(t!("History")).open(&mut open).default_size([460.0, 480.0]).show(ctx, |ui| {
            // 「操作の記録」と「版」のタブ。
            if let Some(tab) = super::icons::panel_tabs(ui, &[t!("Operation history"), t!("Versions")], self.version_ui.tab) {
                self.version_ui.tab = tab;
            }
            if self.version_ui.tab == 1 {
                self.versions_tab(ui);
                return;
            }
            ui.horizontal(|ui| {
                ui.checkbox(&mut record, t!("Record operations")).on_hover_text(t!("The history is saved in the project file. To give the file to someone without it, use File → Save Without History…"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(t!("{} item(s)", self.log.len()));
                });
            });
            ui.separator();
            if self.log.is_empty() {
                ui.weak(t!("Nothing recorded yet. Changes to layers, adjustments, masks and artwork appear here, newest first."));
                return;
            }
            let row = ui.spacing().interact_size.y;
            let n = self.log.len();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row, n, |ui, range| {
                for k in range {
                    let entry = &self.log[n - 1 - k];
                    let time = entry.time.get(11..16).unwrap_or(&entry.time);
                    ui.horizontal(|ui| {
                        ui.weak(time);
                        ui.label(egui::RichText::new(actor_label(&entry.actor)).color(super::theme::MUTED));
                        let response = ui.label(command_label(&entry.command));
                        let target = targets(&entry.args);
                        if !target.is_empty() {
                            ui.weak(target);
                        }
                        let detail = if entry.args.is_null() { String::new() } else { entry.args.to_string() };
                        response.on_hover_text(format!("{}\n{}\n{detail}", entry.time, entry.command));
                    });
                }
            });
        });
        if record != self.settings.record_log {
            self.settings.record_log = record;
            if let Err(e) = self.settings.save() {
                self.say(e);
            }
        }
        if !open {
            self.log_open = false;
        }
    }
}
