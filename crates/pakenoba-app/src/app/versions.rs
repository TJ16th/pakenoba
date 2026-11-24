//! 版を残す（docs/agent-and-mcp.md の 4 章）。
//!
//! - 「版を残す…」で、名前を付けて、いまの案件を残す。エージェントに頼む前には、自動で残す（新しい 10 個まで）。
//! - 版は案件のファイルの `versions/` に入る。素材とマスクは中身の印で入るので、いまと同じ中身ならファイルは増えない。
//! - 版は「開いて見る」（新しいタブ）と「この版に戻す」（元に戻せる。戻す前の状態も自動の版に残す）ができる。

use eframe::egui;
use pakenoba_core::project::{AUTO_VERSIONS, Project, Version};
use pakenoba_i18n::t;

use super::App;

/// 版の窓の状態。
#[derive(Default)]
pub(super) struct VersionUi {
    /// 名前を付けて残す窓（打っている名前）。
    pub naming: Option<String>,
    /// 消すことを確かめている版。
    pub deleting: Option<usize>,
    /// 履歴の窓のタブ（0 は操作の記録、1 は版）。
    pub tab: usize,
}

impl App {
    /// いまの案件を、版の形にする（記録と版は持たない）。
    fn version_now(&mut self) -> Option<Project> {
        match self.project() {
            Ok(mut project) => {
                (project.log, project.versions) = (Vec::new(), Vec::new());
                Some(project)
            }
            Err(e) => {
                self.say(t!("Could not keep a version: {e}"));
                None
            }
        }
    }

    /// 版を残す。自動の版は、新しい `AUTO_VERSIONS` 個だけ残す。
    pub(super) fn keep_version(&mut self, name: &str, auto: bool) -> bool {
        let Some(project) = self.version_now() else { return false };
        // いちばん新しい自動の版と同じ中身なら、増やさない（続けて頼んだときなど）。
        if auto && self.versions.last().is_some_and(|v| v.auto && v.project == project) {
            return true;
        }
        self.versions.push(Version { name: name.to_owned(), time: super::file::now_local(), auto, project });
        while self.versions.iter().filter(|v| v.auto).count() > AUTO_VERSIONS {
            let oldest = self.versions.iter().position(|v| v.auto).unwrap_or(0);
            self.versions.remove(oldest);
        }
        true
    }

    /// 版を、新しいタブで開いて見る。
    fn open_version(&mut self, index: usize) {
        let Some(version) = self.versions.get(index).cloned() else { return };
        if !self.open_tab() {
            return;
        }
        let problems = self.apply_project(version.project);
        self.file.suggest = t!("{} (version)", version.name);
        self.say(if problems.is_empty() { t!("Opened the version \"{}\" in a new tab. Save it to keep it as its own project.", version.name) } else { problems.join(" / ") });
    }

    /// 案件を、版の状態に戻す。元に戻せる。戻す前の状態も、自動の版に残す。
    fn restore_version(&mut self, index: usize) {
        let Some(version) = self.versions.get(index).cloned() else { return };
        self.keep_version(&t!("Before going back to \"{}\"", version.name), true);
        self.checkpoint();
        // 開き直すのと同じ手順で中身を入れ替え、元に戻すための記録、操作の記録、版の一覧は残す。
        let history = std::mem::take(&mut self.history);
        let log = std::mem::take(&mut self.log);
        let versions = std::mem::take(&mut self.versions);
        let path = self.file.path.clone();
        let saved = self.file.saved;
        let problems = self.apply_project(version.project);
        (self.history, self.log, self.versions) = (history, log, versions);
        (self.file.path, self.file.saved) = (path, saved);
        self.log_op("person", "version.restore", serde_json::json!({ "name": version.name, "time": version.time }));
        self.say(if problems.is_empty() { t!("Went back to the version \"{}\". Ctrl+Z undoes it.", version.name) } else { problems.join(" / ") });
    }

    /// 版の一覧（履歴の窓の「版」のタブ）。
    pub(super) fn versions_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button(t!("Keep a version…")).on_hover_text(t!("Keep the project as it is now, with a name")).clicked() {
                self.version_ui.naming = Some(String::new());
            }
            ui.weak(t!("Before each request to the agent, a version is kept automatically (the newest {AUTO_VERSIONS})."));
        });
        ui.separator();
        if self.versions.is_empty() {
            ui.weak(t!("No versions yet."));
            return;
        }
        let (mut open, mut restore, mut delete) = (None, None, None);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (index, version) in self.versions.iter().enumerate().rev() {
                egui::Frame::new().fill(super::theme::SUNKEN).corner_radius(4.0).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(&version.name);
                                if version.auto {
                                    ui.label(egui::RichText::new(t!("auto")).small().color(super::theme::FAINT));
                                }
                            });
                            let when = super::file::show_time(&version.time).unwrap_or_default();
                            let layers = version.project.decals.len();
                            ui.small(egui::RichText::new(t!("{when} · {layers} layer(s)")).color(super::theme::MUTED));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("×").on_hover_text(t!("Delete this version")).clicked() {
                                delete = Some(index);
                            }
                            if ui.button(t!("Go back to this")).on_hover_text(t!("Make the project like this version (Ctrl+Z undoes it)")).clicked() {
                                restore = Some(index);
                            }
                            if ui.button(t!("Open")).on_hover_text(t!("Open this version in a new tab to compare")).clicked() {
                                open = Some(index);
                            }
                        });
                    });
                });
            }
        });
        if let Some(index) = open {
            self.open_version(index);
        }
        if let Some(index) = restore {
            self.restore_version(index);
        }
        if let Some(index) = delete {
            self.version_ui.deleting = Some(index);
        }
    }

    /// 版の窓（名前を付けて残す、消す確認）。
    pub(super) fn version_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(mut name) = self.version_ui.naming.take() {
            let buttons = [super::dialog::Button::accept(t!("Keep"), !name.trim().is_empty()), super::dialog::Button::cancel(t!("Cancel"))];
            let picked = super::dialog::show(ctx, "version-name", t!("Keep a version"), 380.0, &buttons, |ui| {
                ui.label(egui::RichText::new(t!("Name")).color(super::theme::MUTED));
                let field = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).hint_text(t!("e.g. First draft for the client")));
                if !field.has_focus() && name.is_empty() {
                    field.request_focus();
                }
                ui.small(egui::RichText::new(t!("Named versions stay until you delete them. They are saved in the project file.")).color(super::theme::FAINT));
            });
            match picked {
                Some(0) => {
                    if self.keep_version(name.trim(), false) {
                        self.say(t!("Kept the version \"{}\".", name.trim()));
                    }
                }
                Some(_) => {}
                None => self.version_ui.naming = Some(name),
            }
        }
        if let Some(index) = self.version_ui.deleting {
            let Some(version) = self.versions.get(index) else {
                self.version_ui.deleting = None;
                return;
            };
            let title = t!("Delete the version \"{}\"?", version.name);
            let buttons = [super::dialog::Button::accept(t!("Delete"), true), super::dialog::Button::cancel(t!("Cancel"))];
            let picked = super::dialog::show(ctx, "version-delete", &title, 380.0, &buttons, |ui| {
                ui.label(t!("A deleted version cannot be brought back."));
            });
            match picked {
                Some(0) => {
                    self.versions.remove(index);
                    self.version_ui.deleting = None;
                }
                Some(_) => self.version_ui.deleting = None,
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    #[test]
    fn 版を残し_戻すと元に戻せて_自動の版は10個まで() {
        let mut app = App::new(&Initial::default());
        let before = app.decals.len();
        assert!(app.keep_version("はじめ", false));
        app.run_command("person", "decal.add", serde_json::json!({ "builtin": "band" })).unwrap();
        assert_eq!(app.decals.len(), before + 1);
        app.restore_version(0);
        assert_eq!(app.decals.len(), before);
        // 戻す前の状態も、自動の版に残っている。
        assert_eq!(app.versions.len(), 2);
        assert!(app.versions[1].auto && app.versions[1].project.decals.len() == before + 1);
        // 戻したのも、Ctrl+Z で元に戻せる。
        app.undo();
        assert_eq!(app.decals.len(), before + 1);
        // 自動の版は、新しい 10 個だけ。名前を付けた版は残る。
        for k in 0..15 {
            app.run_command("person", "layer.opacity.set", serde_json::json!({ "ids": [app.decals[0].layer.id], "opacity": 50 + k })).unwrap();
            app.keep_version("自動", true);
        }
        assert_eq!(app.versions.iter().filter(|v| v.auto).count(), AUTO_VERSIONS);
        assert_eq!(app.versions[0].name, "はじめ");
        // 同じ中身なら、自動の版は増やさない。
        let count = app.versions.len();
        app.keep_version("自動", true);
        assert_eq!(app.versions.len(), count);
        // 版は案件に入り、保存して開くと戻る。
        let project = app.project().unwrap();
        assert_eq!(project.versions.len(), count);
    }
}
