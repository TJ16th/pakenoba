//! 文字の道具と、文字の柄の設定。
//!
//! 文字の道具（T）で箱や展開図を押すと、そこに文字の柄を置いて、右の欄で文字を打てるようにする。
//! 文字と書式（フォント、大きさ、色、揃え、字間、行送り）を変えると、絵を描き直す。拡大や縮小をしていれば、その倍率を保つ。

use std::sync::Arc;

use eframe::egui;

use pakenoba_core::project::{TEXT_LEADING, TEXT_SIZE, TEXT_SPACING, TextAlign, TextSpec};
use pakenoba_core::surface::Decal;
use pakenoba_core::template::P2;

use super::App;
use crate::paint::{Picture, Placed};

/// 新しく置く文字の、はじめの言葉。
const FIRST_WORDS: &str = "テキスト";

/// はじめの書式（フォントは、見つかったものを入れる）。
pub(super) fn default_style() -> TextSpec {
    TextSpec { text: String::new(), font: String::new(), size: 8.0, color: [20, 20, 20], align: TextAlign::Left, spacing: 0.0, leading: 1.5 }
}

/// 書式の操作（フォント、大きさ、色、揃え、字間、行送り）。変えたら真。
fn style_controls(ui: &mut egui::Ui, spec: &mut TextSpec, families: &[String], used: &[String], compact: bool) -> bool {
    let mut changed = false;
    let font_label = if spec.font.is_empty() { "（フォント）".to_owned() } else { spec.font.clone() };
    egui::ComboBox::from_id_salt(if compact { "text-font-bar" } else { "text-font" }).width(if compact { 170.0 } else { 200.0 }).height(360.0).selected_text(font_label).show_ui(ui, |ui| {
        // この案件で使っているフォントを先に、そのあとに、入っているすべてのフォント。
        if !used.is_empty() {
            ui.weak("この案件で使っているフォント");
            for name in used {
                changed |= ui.selectable_value(&mut spec.font, name.clone(), name).changed();
            }
            ui.separator();
        }
        for name in families {
            changed |= ui.selectable_value(&mut spec.font, name.clone(), name).changed();
        }
    });
    if !compact {
        ui.end_row();
        ui.label("大きさ");
    }
    changed |= ui.add(egui::DragValue::new(&mut spec.size).range(TEXT_SIZE).speed(0.1).max_decimals(1).suffix(" mm")).on_hover_text("文字の大きさ（全角 1 文字の高さ）").changed();
    if !compact {
        ui.end_row();
        ui.label("色");
    }
    changed |= ui.color_edit_button_srgb(&mut spec.color).on_hover_text("文字の色").changed();
    if !compact {
        ui.end_row();
        ui.label("揃え");
    }
    ui.horizontal(|ui| {
        for (align, label) in [(TextAlign::Left, "左"), (TextAlign::Center, "中央"), (TextAlign::Right, "右")] {
            changed |= ui.selectable_value(&mut spec.align, align, label).on_hover_text(format!("{label}揃え")).changed();
        }
    });
    if !compact {
        ui.end_row();
        ui.label("字間");
        changed |= ui.add(egui::DragValue::new(&mut spec.spacing).range(TEXT_SPACING).speed(5.0).max_decimals(0)).on_hover_text("全角の 1/1000 単位。0 がふつう").changed();
        ui.end_row();
        ui.label("行送り");
        changed |= ui.add(egui::DragValue::new(&mut spec.leading).range(TEXT_LEADING).speed(0.01).max_decimals(2).suffix(" 倍")).on_hover_text("文字の大きさの何倍で、次の行へ送るか").changed();
        ui.end_row();
    }
    changed
}

impl App {
    /// 文字から、柄の絵を描く。求めたフォントがなかったら、そう知らせる。
    pub(super) fn text_picture(&mut self, spec: &TextSpec) -> Result<Picture, String> {
        let art = self.fonts.render(spec)?;
        Ok(Picture::Text(Arc::new(art)))
    }

    /// 新しく置く文字の書式。フォントが空なら、見つかったフォントを入れる。
    fn new_style(&mut self) -> TextSpec {
        if self.text_style.font.is_empty() {
            self.text_style.font = self.fonts.default_family().unwrap_or_default();
        }
        TextSpec { text: FIRST_WORDS.to_owned(), ..self.text_style.clone() }
    }

    /// 文字の柄を、パネル `panel` の、展開図の上の点 `at` を中心に置き、選んで、文字を打てるようにする。
    pub(super) fn add_text_at(&mut self, panel: usize, at: P2, back: bool) {
        let spec = self.new_style();
        let picture = match self.text_picture(&spec) {
            Ok(picture) => picture,
            Err(e) => return self.say(format!("文字を置けませんでした: {e}")),
        };
        let Picture::Text(art) = &picture else { return };
        let size = art.size_mm;
        self.checkpoint();
        let mut decal = Decal { panel, at: [0.5, 0.5], size, angle: 0.0 };
        decal.set_center(&self.template, at);
        self.decals.push(Placed { decal, picture, back, layer: Default::default() });
        self.select_only(Some(self.decals.len() - 1));
        self.focus_text = true;
        self.say("文字を置きました。右の欄で文字を打ちます。");
    }

    /// この案件で使っているフォント。
    fn used_fonts(&self) -> Vec<String> {
        let mut used: Vec<String> = self.decals.iter().filter_map(|d| if let Picture::Text(art) = &d.picture { Some(art.spec.font.clone()) } else { None }).collect();
        used.sort();
        used.dedup();
        used
    }

    /// 文字の柄 `index` の文字か書式を `spec` に変えて、描き直す。大きさは、いまの倍率を保つ。
    pub(super) fn retype(&mut self, index: usize, spec: TextSpec) {
        let Some(Picture::Text(old)) = self.decals.get(index).map(|d| d.picture.clone()) else { return };
        if old.spec == spec {
            return;
        }
        match self.text_picture(&spec) {
            Ok(picture) => {
                let Picture::Text(art) = &picture else { return };
                let scale = [self.decals[index].size[0] / old.size_mm[0].max(1e-3), self.decals[index].size[1] / old.size_mm[1].max(1e-3)];
                let size = [art.size_mm[0] * scale[0], art.size_mm[1] * scale[1]];
                let template = self.template.clone();
                let decal = &mut self.decals[index];
                let center = decal.center(&template);
                decal.size = size;
                decal.set_center(&template, center);
                decal.picture = picture;
                (self.art_dirty, self.back_dirty) = (true, true);
            }
            Err(e) => self.say(format!("文字を描けませんでした: {e}")),
        }
    }

    /// 右の欄の、文字の柄の設定（文字と書式）。プロパティの欄の上に出す。
    pub(super) fn text_properties(&mut self, ui: &mut egui::Ui, index: usize, locked: bool) {
        let Some(Picture::Text(art)) = self.decals.get(index).map(|d| d.picture.clone()) else { return };
        let earlier = self.snapshot();
        let mut spec = art.spec.clone();
        let mut changed = false;
        ui.add_enabled_ui(!locked, |ui| {
            let edit = ui.add(egui::TextEdit::multiline(&mut spec.text).desired_rows(2).desired_width(f32::INFINITY).hint_text("文字を打つ（改行で次の行）"));
            if std::mem::take(&mut self.focus_text) {
                edit.request_focus();
                // はじめの言葉は、すぐ打ち替えられるよう、全体を選んでおく。
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit.id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(spec.text.chars().count()))));
                    state.store(ui.ctx(), edit.id);
                }
            }
            changed |= edit.changed();
            let families = self.fonts.families().to_vec();
            let used = self.used_fonts();
            egui::Grid::new("text-style").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("フォント");
                changed |= style_controls(ui, &mut spec, &families, &used, false);
            });
        });
        if art.font_used != art.spec.font {
            ui.colored_label(ui.visuals().warn_fg_color, format!("「{}」がこの PC にないので、{} で描いています。", art.spec.font, art.font_used));
        }
        if changed {
            self.retype(index, spec.clone());
            // 次に置く文字も、同じ書式にする。
            self.text_style = TextSpec { text: String::new(), ..spec };
            // 打っているあいだは、1 回ぶんにまとめる。
            if !self.editing {
                self.history.record(earlier);
            }
            self.editing = true;
        }
        ui.add_space(4.0);
    }

    /// 上の帯（文字の道具のとき）。選んでいるのが文字なら、その書式を変える。そうでなければ、次に置く文字の書式。
    pub(super) fn text_bar(&mut self, ui: &mut egui::Ui) {
        let target = self.selected.filter(|&i| matches!(self.decals.get(i).map(|d| &d.picture), Some(Picture::Text(_))) && !self.decals[i].layer.locked);
        let mut spec = match target.and_then(|i| if let Picture::Text(art) = &self.decals[i].picture { Some(art.spec.clone()) } else { None }) {
            Some(spec) => spec,
            None => {
                if self.text_style.font.is_empty() {
                    self.text_style.font = self.fonts.default_family().unwrap_or_default();
                }
                self.text_style.clone()
            }
        };
        let families = self.fonts.families().to_vec();
        let used = self.used_fonts();
        let earlier = self.snapshot();
        let changed = style_controls(ui, &mut spec, &families, &used, true);
        ui.separator();
        ui.weak(if target.is_some() { "選んでいる文字の書式" } else { "箱か展開図を押すと、そこに文字を置きます" });
        if changed {
            if let Some(index) = target {
                self.retype(index, spec.clone());
                if !self.editing {
                    self.history.record(earlier);
                }
                self.editing = true;
            }
            self.text_style = TextSpec { text: String::new(), ..spec };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    #[test]
    fn 文字を置いて打ち替えると_倍率を保って描き直し_保存して戻る() {
        let mut app = App::new(&Initial::default());
        if app.fonts.families().is_empty() {
            eprintln!("フォントがない環境なので、確かめない");
            return;
        }
        let panel = app.template.panels.iter().position(|p| p.name == "正面").unwrap();
        let at = Decal { panel, at: [0.5, 0.5], size: [1.0, 1.0], angle: 0.0 }.center(&app.template);
        app.add_text_at(panel, at, false);
        let index = app.decals.len() - 1;
        assert!(matches!(&app.decals[index].picture, Picture::Text(art) if art.spec.text == FIRST_WORDS));
        assert!(app.focus_text, "置いたら、すぐ打てるようにする");
        // 2 倍に広げてから、文字を長くする。高さの倍率は保つ。
        let natural = app.decals[index].size;
        app.decals[index].decal.size = [natural[0] * 2.0, natural[1] * 2.0];
        let Picture::Text(art) = app.decals[index].picture.clone() else { unreachable!() };
        app.retype(index, TextSpec { text: "テキストを長くした".to_owned(), ..art.spec.clone() });
        let now = app.decals[index].size;
        assert!((now[1] - natural[1] * 2.0).abs() < 0.2, "高さの倍率が変わった: {natural:?} → {now:?}");
        assert!(now[0] > natural[0] * 3.0, "文字を長くしたのに、幅が増えない");
        assert!((app.decals[index].center(&app.template)[0] - at[0]).abs() < 1e-3, "中心は動かない");
        // 保存して開くと、同じ文字と書式で描き直される。
        let project = app.project().unwrap();
        let mut again = App::new(&Initial::default());
        let problems = again.apply_project(project);
        assert!(problems.is_empty(), "{problems:?}");
        let Picture::Text(back) = &again.decals[index].picture else { panic!("文字の柄にならない") };
        assert_eq!(back.spec.text, "テキストを長くした");
        assert_eq!(again.decals[index].size, now);
        // 元に戻すと、置く前に戻る。
        app.undo();
        app.undo();
        assert_eq!(app.decals.len(), index);
    }

    #[test]
    fn ないフォントの文字は_別のフォントで描いて知らせる() {
        let mut app = App::new(&Initial::default());
        if app.fonts.families().is_empty() {
            return;
        }
        let mut project = app.project().unwrap();
        let panel = app.template.panels.iter().position(|p| p.name == "正面").unwrap();
        project.decals.push(pakenoba_core::project::Placement {
            decal: Decal { panel, at: [0.5, 0.5], size: [30.0, 10.0], angle: 0.0 },
            back: false,
            source: pakenoba_core::project::Source::Text(TextSpec { text: "箱".to_owned(), font: "この世にないフォント".to_owned(), ..default_style() }),
            layer: Default::default(),
        });
        let problems = app.apply_project(project);
        assert_eq!(app.decals.len(), 1);
        assert!(problems.iter().any(|p| p.contains("この世にないフォント")), "{problems:?}");
        // 保存しても、求めたフォントの名前は残す（そのフォントがある PC では、そのフォントで描く）。
        let Some(pakenoba_core::project::Source::Text(spec)) = app.project().unwrap().decals.first().map(|d| d.source.clone()) else { panic!() };
        assert_eq!(spec.font, "この世にないフォント");
    }
}
