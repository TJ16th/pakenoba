//! 加工のレイヤー（箔押し）。
//!
//! 形は、つないだ柄の形を借りる（柄を動かすと、加工も付いてくる）。柄を隠しても、形には使う。
//! 描くときは、表と裏それぞれに「加工の版の絵」を作って GPU へ送る（R が箔の量、G が箔の種類）。
//! くわしくは `docs/finishes.md`。

use pakenoba_i18n::t;

use eframe::egui;
use pakenoba_core::material::{Finish, Foil};
use pakenoba_core::project::{Blend, Layer};
use serde_json::{Value, json};

use super::App;
use crate::paint::{Canvas, Marks, is_artwork, paint};

/// 展開図の表示で、箔の場所を見せる色。
pub(crate) fn foil_swatch(foil: Foil) -> [u8; 3] {
    match foil {
        Foil::Gold | Foil::MatteGold => [212, 175, 55],
        Foil::Silver | Foil::MatteSilver => [192, 196, 202],
        Foil::Copper => [196, 120, 84],
        Foil::RedGold => [214, 140, 70],
        Foil::Hologram => [170, 200, 230],
    }
}

/// 加工の版の絵の G に入れる、箔の種類の値。
pub(crate) fn foil_code(foil: Foil) -> u8 {
    (foil.index() * 32 + 16) as u8
}

impl App {
    /// 加工がつながっている柄の番号。つないだ柄がなければ None。
    pub(super) fn finish_link(&self, index: usize) -> Option<usize> {
        let link = self.finishes.get(index)?.link;
        self.decals.iter().position(|d| d.layer.id == link && is_artwork(&d.picture))
    }

    /// 画面に出す名前（名前がなければ「金の箔（ロゴ）」）。
    pub(super) fn finish_label(&self, index: usize) -> String {
        let finish = &self.finishes[index];
        if let Some(name) = &finish.name {
            return name.clone();
        }
        match self.finish_link(index) {
            Some(i) => format!("{}（{}）", finish.foil.label(), self.layer_label(i)),
            None => finish.foil.label().to_owned(),
        }
    }

    /// 加工の版の絵（`back` が真なら紙の裏）。R が箔の量、G が箔の種類。加工がなければ None。
    pub(super) fn finish_map(&self, back: bool) -> Option<Vec<u8>> {
        let (w, h) = (self.sheet.w, self.sheet.h);
        let mut map: Option<Vec<u8>> = None;
        for index in 0..self.finishes.len() {
            let finish = &self.finishes[index];
            let Some(link) = self.finish_link(index).filter(|&i| !finish.hidden && self.decals[i].back == back) else { continue };
            // 柄の形だけを使う。隠していても、不透明度や描画モードを変えていても、形はそのまま。
            let mut shape = self.decals[link].clone();
            shape.layer = Layer { id: shape.layer.id, mask: shape.layer.mask.clone(), opacity: 100, fill: 100, blend: Blend::Normal, ..Layer::default() };
            let mut layer = vec![0u8; w * h * 4];
            paint(&self.surface, &self.template, std::slice::from_ref(&shape), Marks::default(), back, Canvas::screen(w, h, &mut layer));
            let map = map.get_or_insert_with(|| vec![0u8; w * h * 4]);
            let code = foil_code(finish.foil);
            for (m, p) in map.chunks_exact_mut(4).zip(layer.chunks_exact(4)) {
                if p[3] > 0 {
                    // 上に重ねた加工が勝つ（量は大きいほう）。
                    m[0] = m[0].max(p[3]);
                    m[1] = code;
                    m[3] = 255;
                }
            }
        }
        map
    }

    /// 展開図の表示の絵に、箔の場所を、その箔の色で重ねる。
    pub(super) fn overlay_finishes(&self, flat: &mut [u8], map: &[u8]) {
        for (p, m) in flat.chunks_exact_mut(4).zip(map.chunks_exact(4)) {
            if m[0] == 0 {
                continue;
            }
            let foil = Foil::ALL[(m[1] as usize / 32).min(Foil::ALL.len() - 1)];
            let swatch = foil_swatch(foil);
            let a = m[0] as f32 / 255.0;
            for c in 0..3 {
                p[c] = (p[c] as f32 * (1.0 - a) + swatch[c] as f32 * a).round() as u8;
            }
            p[3] = p[3].max(m[0]);
        }
    }

    /// 印（id）のない加工に、印を付ける。柄のレイヤーの印と重ならない数にする。
    pub(super) fn ensure_finish_ids(&mut self) {
        let mut next = self.decals.iter().map(|d| d.layer.id).chain(self.finishes.iter().map(|f| f.id)).max().unwrap_or(0);
        let mut seen = std::collections::HashSet::new();
        for finish in &mut self.finishes {
            if finish.id == 0 || !seen.insert(finish.id) || self.decals.iter().any(|d| d.layer.id == finish.id) {
                next += 1;
                finish.id = next;
                seen.insert(next);
            }
        }
    }

    fn finish_index(&self, id: i64) -> Result<usize, String> {
        self.finishes.iter().position(|f| i64::from(f.id) == id).ok_or_else(|| t!("There is no finish layer with id {id}"))
    }

    pub(super) fn finishes_json(&self) -> Value {
        Value::Array(
            self.finishes
                .iter()
                .enumerate()
                .map(|(i, f)| json!({ "id": f.id, "kind": "foil", "foil": f.foil.key(), "artwork": f.link, "artwork_found": self.finish_link(i).is_some(), "name": f.name, "hidden": f.hidden }))
                .collect(),
        )
    }

    /// 加工のコマンド。知らない名前なら None。
    pub(super) fn dispatch_finish(&mut self, name: &str, args: &Value) -> Option<Result<Value, String>> {
        let result = match name {
            "finish.add" => (|| -> Result<Value, String> {
                self.ensure_ids();
                let id = args.get("id").and_then(Value::as_i64).ok_or_else(|| t!("\"id\" is missing").to_owned())?;
                let index = self.index_of(id)?;
                if !is_artwork(&self.decals[index].picture) {
                    return Err(t!("Foil can only be added to artwork (an image, text or shape), not to a group or adjustment layer").to_owned());
                }
                let foil = match args.get("foil").and_then(Value::as_str) {
                    Some(key) => Foil::from_key(key).ok_or_else(|| t!("Unknown foil \"{key}\""))?,
                    None => Foil::Gold,
                };
                self.checkpoint();
                self.finishes.push(Finish { id: 0, foil, link: self.decals[index].layer.id, name: None, hidden: false });
                self.ensure_finish_ids();
                let new = self.finishes.len() - 1;
                self.select_finish(Some(new));
                (self.art_dirty, self.back_dirty) = (true, true);
                Ok(json!({ "id": self.finishes[new].id }))
            })(),
            "finish.set" => (|| -> Result<Value, String> {
                let index = self.finish_index(args.get("id").and_then(Value::as_i64).ok_or_else(|| t!("\"id\" is missing").to_owned())?)?;
                let mut next = self.finishes[index].clone();
                if let Some(key) = args.get("foil").and_then(Value::as_str) {
                    next.foil = Foil::from_key(key).ok_or_else(|| t!("Unknown foil \"{key}\""))?;
                }
                if let Some(hidden) = args.get("visible").and_then(Value::as_bool) {
                    next.hidden = !hidden;
                }
                if let Some(name) = args.get("name").and_then(Value::as_str) {
                    let name = name.trim();
                    next.name = (!name.is_empty()).then(|| name.to_owned());
                }
                if next != self.finishes[index] {
                    self.checkpoint();
                    self.finishes[index] = next;
                    (self.art_dirty, self.back_dirty) = (true, true);
                }
                Ok(json!({ "id": self.finishes[index].id }))
            })(),
            "finish.delete" => (|| -> Result<Value, String> {
                let index = self.finish_index(args.get("id").and_then(Value::as_i64).ok_or_else(|| t!("\"id\" is missing").to_owned())?)?;
                self.checkpoint();
                self.finishes.remove(index);
                self.finish_selected = None;
                (self.art_dirty, self.back_dirty) = (true, true);
                Ok(json!({ "deleted": 1 }))
            })(),
            _ => return None,
        };
        Some(result)
    }

    /// 加工を選ぶ（柄の選びは外す。プロパティに加工の設定を出す）。
    pub(super) fn select_finish(&mut self, index: Option<usize>) {
        self.finish_selected = index;
        if index.is_some() {
            self.selected = None;
            self.extra.clear();
            (self.art_dirty, self.back_dirty) = (true, true);
        }
    }

    /// 選んでいる柄に、箔押しを付ける。
    pub(super) fn add_foil(&mut self, foil: Foil) {
        let Some(index) = self.selected.filter(|&i| i < self.decals.len()) else { return };
        self.ensure_ids();
        let id = self.decals[index].layer.id;
        self.person("finish.add", json!({ "id": id, "foil": foil.key() }));
    }

    /// 加工を選んだときのプロパティ。
    pub(super) fn finish_properties(&mut self, ui: &mut egui::Ui, index: usize) {
        let id = self.finishes[index].id;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
            super::icons::draw_foil(ui.painter(), rect.center(), egui::Stroke::new(1.2, super::theme::TEXT));
            ui.label(egui::RichText::new(self.finish_label(index)).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.weak(t!("Foil stamping")));
        });
        let mut foil = self.finishes[index].foil;
        egui::Grid::new("finish").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(egui::RichText::new(t!("Foil")).color(super::theme::MUTED));
            egui::ComboBox::from_id_salt("foil").width(170.0).selected_text(foil.label()).show_ui(ui, |ui| {
                for f in Foil::ALL {
                    ui.selectable_value(&mut foil, f, f.label());
                }
            });
            ui.end_row();
            ui.label(egui::RichText::new(t!("Shape from")).color(super::theme::MUTED));
            match self.finish_link(index) {
                Some(link) => {
                    let label = self.layer_label(link);
                    if ui.link(label).on_hover_text(t!("Select this artwork")).clicked() {
                        self.select_only(Some(link));
                        self.finish_selected = None;
                    }
                }
                None => {
                    ui.colored_label(super::theme::WARN, t!("The artwork it was linked to is gone"));
                }
            }
            ui.end_row();
        });
        if foil != self.finishes[index].foil {
            self.person("finish.set", json!({ "id": id, "foil": foil.key() }));
        }
        ui.small(t!("The foil takes the shape of the linked artwork and moves with it. Hide the artwork's layer to stamp foil without ink."));
        if ui.button(t!("Remove the foil")).on_hover_text(t!("Ctrl+Z undoes this")).clicked() {
            self.person("finish.delete", json!({ "id": id }));
        }
    }
}
