//! 加工のレイヤー（箔押し）。
//!
//! 形は、つないだ柄の形を借りる（柄を動かすと、加工も付いてくる）。柄を隠しても、形には使う。
//! 描くときは、表と裏それぞれに「加工の版の絵」を作って GPU へ送る（R が箔の量、G が箔の種類）。
//! くわしくは `docs/finishes.md`。

use pakenoba_i18n::t;

use eframe::egui;
use pakenoba_core::material::{EMBOSS_EDGE, EMBOSS_HEIGHT, Finish, FinishKind, Foil};
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

/// 加工の版の絵の A に入れる高さの、いちばん高い値（mm）。A は 128 が高さ 0。
pub(crate) const MAX_RELIEF_MM: f32 = 2.0;

/// 横と縦に、幅 `r` の箱ぼかしを 2 回かける（縁の坂を作る）。
fn soften(values: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let mut line = Vec::new();
    for _ in 0..2 {
        for y in 0..h {
            line.clear();
            line.extend_from_slice(&values[y * w..(y + 1) * w]);
            blur_line(&line, &mut values[y * w..(y + 1) * w], r);
        }
        let mut column = vec![0.0f32; h];
        let mut out = vec![0.0f32; h];
        for x in 0..w {
            for y in 0..h {
                column[y] = values[y * w + x];
            }
            blur_line(&column, &mut out, r);
            for y in 0..h {
                values[y * w + x] = out[y];
            }
        }
    }
}

fn blur_line(src: &[f32], dst: &mut [f32], r: usize) {
    let n = src.len();
    let at = |i: isize| if i < 0 || i >= n as isize { 0.0 } else { src[i as usize] };
    let width = (2 * r + 1) as f32;
    let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
    for (i, out) in dst.iter_mut().enumerate().take(n) {
        *out = sum / width;
        sum += at(i as isize + r as isize + 1) - at(i as isize - r as isize);
    }
}

/// 加工の版の絵の G に入れる、箔の種類の値。
pub(crate) fn foil_code(foil: Foil) -> u8 {
    (foil.index() * 32 + 16) as u8
}

/// コマンドの引数から、加工の値を変える（箔の種類、うねり、エンボスの高さ、へこませるか、縁の幅）。
fn apply_finish_args(finish: &mut Finish, args: &Value) -> Result<(), String> {
    if let Some(key) = args.get("foil").and_then(Value::as_str) {
        finish.foil = Foil::from_key(key).ok_or_else(|| t!("Unknown foil \"{key}\""))?;
    }
    if let Some(v) = args.get("wave") {
        finish.wave = v.as_u64().filter(|&w| w <= 100).ok_or_else(|| t!("\"wave\" is a number from 0 to 100").to_owned())? as u8;
    }
    let mm = |key: &str, range: &std::ops::RangeInclusive<f32>| -> Result<Option<f32>, String> {
        match args.get(key) {
            None => Ok(None),
            Some(v) => v.as_f64().filter(|v| v.is_finite()).map(|v| Some((v as f32).clamp(*range.start(), *range.end()))).ok_or_else(|| t!("\"{key}\" (a number) is missing")),
        }
    };
    if let Some(v) = mm("height", &EMBOSS_HEIGHT)? {
        finish.height = v;
    }
    if let Some(v) = mm("edge", &EMBOSS_EDGE)? {
        finish.edge = v;
    }
    if let Some(v) = args.get("deboss").and_then(Value::as_bool) {
        finish.deboss = v;
    }
    Ok(())
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
        let kind = match (finish.kind, finish.deboss) {
            (FinishKind::Foil, _) => finish.foil.label(),
            (FinishKind::Emboss, false) => t!("Emboss"),
            (FinishKind::Emboss, true) => t!("Deboss"),
        };
        match self.finish_link(index) {
            Some(i) => format!("{kind}（{}）", self.layer_label(i)),
            None => kind.to_owned(),
        }
    }

    /// 加工の版の絵（`back` が真なら紙の裏）。R が箔の量、G が箔の種類、B が箔のうねり、
    /// A が浮き出しの高さ（128 が 0、255 が `MAX_RELIEF_MM`、0 がその逆）。加工がなければ None。
    pub(super) fn finish_map(&self, back: bool) -> Option<Vec<u8>> {
        let (w, h) = (self.sheet.w, self.sheet.h);
        let mut map: Option<Vec<u8>> = None;
        let mut relief: Option<Vec<f32>> = None;
        for index in 0..self.finishes.len() {
            let finish = &self.finishes[index];
            let Some(link) = self.finish_link(index).filter(|&i| !finish.hidden && self.decals[i].back == back) else { continue };
            // 柄の形だけを使う。隠していても、不透明度や描画モードを変えていても、形はそのまま。
            let mut shape = self.decals[link].clone();
            shape.layer = Layer { id: shape.layer.id, mask: shape.layer.mask.clone(), opacity: 100, fill: 100, blend: Blend::Normal, ..Layer::default() };
            let mut layer = vec![0u8; w * h * 4];
            paint(&self.surface, &self.template, std::slice::from_ref(&shape), Marks::default(), back, Canvas::screen(w, h, &mut layer));
            let map = map.get_or_insert_with(|| [0u8, 0, 0, 128].repeat(w * h));
            if finish.kind == FinishKind::Emboss {
                // 形を、縁の幅だけぼかして坂にし、高さを掛けて足す（重なったエンボスは足し合わせる）。
                let mut cover: Vec<f32> = layer.chunks_exact(4).map(|p| p[3] as f32 / 255.0).collect();
                let r = (finish.edge * crate::art::PX_PER_MM * 0.5).round() as usize;
                soften(&mut cover, w, h, r.max(1));
                let sign = if finish.deboss { -1.0 } else { 1.0 };
                let heights = relief.get_or_insert_with(|| vec![0.0f32; w * h]);
                for (v, c) in heights.iter_mut().zip(&cover) {
                    *v += sign * finish.height * c;
                }
                continue;
            }
            let code = foil_code(finish.foil);
            let wave = (finish.wave.min(100) as u32 * 255 / 100) as u8;
            for (m, p) in map.chunks_exact_mut(4).zip(layer.chunks_exact(4)) {
                if p[3] > 0 {
                    // 上に重ねた加工が勝つ（量は大きいほう）。
                    m[0] = m[0].max(p[3]);
                    m[1] = code;
                    m[2] = wave;
                }
            }
        }
        if let (Some(map), Some(relief)) = (&mut map, &relief) {
            for (m, v) in map.chunks_exact_mut(4).zip(relief) {
                m[3] = (128.0 + v / MAX_RELIEF_MM * 127.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        map
    }

    /// 展開図の表示の絵に、箔の場所を、その箔の色で重ねる。
    pub(super) fn overlay_finishes(&self, flat: &mut [u8], map: &[u8]) {
        for (p, m) in flat.chunks_exact_mut(4).zip(map.chunks_exact(4)) {
            // 浮き出しは少し明るく、へこみは少し暗く。
            if m[3] != 128 {
                let k = 1.0 + (m[3] as f32 - 128.0) / 127.0 * MAX_RELIEF_MM * 0.35;
                for c in &mut p[..3] {
                    *c = (*c as f32 * k).round().clamp(0.0, 255.0) as u8;
                }
            }
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
                .map(|(i, f)| match f.kind {
                    FinishKind::Foil => json!({ "id": f.id, "kind": "foil", "foil": f.foil.key(), "artwork": f.link, "artwork_found": self.finish_link(i).is_some(), "name": f.name, "hidden": f.hidden, "wave": f.wave }),
                    FinishKind::Emboss => json!({ "id": f.id, "kind": "emboss", "deboss": f.deboss, "height_mm": f.height, "edge_mm": f.edge, "artwork": f.link, "artwork_found": self.finish_link(i).is_some(), "name": f.name, "hidden": f.hidden }),
                })
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
                let link = self.decals[index].layer.id;
                let kind = match args.get("kind").and_then(Value::as_str) {
                    Some(key) => FinishKind::from_key(key).ok_or_else(|| t!("Unknown finish \"{key}\" (foil, emboss)"))?,
                    None => FinishKind::Foil,
                };
                let mut finish = match kind {
                    FinishKind::Foil => Finish::foil(Foil::Gold, link),
                    FinishKind::Emboss => Finish::emboss(link, false),
                };
                apply_finish_args(&mut finish, args)?;
                self.checkpoint();
                self.finishes.push(finish);
                self.ensure_finish_ids();
                let new = self.finishes.len() - 1;
                self.select_finish(Some(new));
                (self.art_dirty, self.back_dirty) = (true, true);
                Ok(json!({ "id": self.finishes[new].id }))
            })(),
            "finish.set" => (|| -> Result<Value, String> {
                let index = self.finish_index(args.get("id").and_then(Value::as_i64).ok_or_else(|| t!("\"id\" is missing").to_owned())?)?;
                let mut next = self.finishes[index].clone();
                apply_finish_args(&mut next, args)?;
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
        self.person("finish.add", json!({ "id": id, "kind": "foil", "foil": foil.key() }));
    }

    /// 選んでいる柄の形で、エンボス（`deboss` が真ならデボス）を付ける。
    pub(super) fn add_emboss(&mut self, deboss: bool) {
        let Some(index) = self.selected.filter(|&i| i < self.decals.len()) else { return };
        self.ensure_ids();
        let id = self.decals[index].layer.id;
        self.person("finish.add", json!({ "id": id, "kind": "emboss", "deboss": deboss }));
    }

    /// 加工を選んだときのプロパティ。
    pub(super) fn finish_properties(&mut self, ui: &mut egui::Ui, index: usize) {
        let id = self.finishes[index].id;
        let kind = self.finishes[index].kind;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
            super::icons::draw_foil(ui.painter(), rect.center(), egui::Stroke::new(1.2, super::theme::TEXT));
            ui.label(egui::RichText::new(self.finish_label(index)).strong());
            let what = if kind == FinishKind::Foil { t!("Foil stamping") } else { t!("Embossing") };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.weak(what));
        });
        egui::Grid::new("finish").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            match kind {
                FinishKind::Foil => {
                    let mut foil = self.finishes[index].foil;
                    ui.label(egui::RichText::new(t!("Foil")).color(super::theme::MUTED));
                    egui::ComboBox::from_id_salt("foil").width(170.0).selected_text(foil.label()).show_ui(ui, |ui| {
                        for f in Foil::ALL {
                            ui.selectable_value(&mut foil, f, f.label());
                        }
                    });
                    ui.end_row();
                    if foil != self.finishes[index].foil {
                        self.person("finish.set", json!({ "id": id, "foil": foil.key() }));
                    }
                    ui.label(egui::RichText::new(t!("Waviness")).color(super::theme::MUTED));
                    let mut wave = self.finishes[index].wave as f32;
                    let r = ui.add(egui::Slider::new(&mut wave, 0.0..=100.0).fixed_decimals(0).suffix(" %")).on_hover_text(t!("Small waves in the pressed foil. 0 is flat like a mirror. More waves make the reflection break up and glitter"));
                    self.finish_slider(r, index, id, "wave", wave);
                    ui.end_row();
                }
                FinishKind::Emboss => {
                    ui.label(egui::RichText::new(t!("Direction")).color(super::theme::MUTED));
                    let deboss = self.finishes[index].deboss;
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::selectable(!deboss, t!("Raise (emboss)"))).clicked() && deboss {
                            self.person("finish.set", json!({ "id": id, "deboss": false }));
                        }
                        if ui.add(egui::Button::selectable(deboss, t!("Press in (deboss)"))).clicked() && !deboss {
                            self.person("finish.set", json!({ "id": id, "deboss": true }));
                        }
                    });
                    ui.end_row();
                    ui.label(egui::RichText::new(t!("Height")).color(super::theme::MUTED));
                    let mut height = self.finishes[index].height;
                    let r = ui.add(egui::Slider::new(&mut height, EMBOSS_HEIGHT).fixed_decimals(2).suffix(" mm")).on_hover_text(t!("How high the pattern is raised (or how deep it is pressed). Usually 0.2 to 1 mm"));
                    self.finish_slider(r, index, id, "height", height);
                    ui.end_row();
                    ui.label(egui::RichText::new(t!("Edge width")).color(super::theme::MUTED));
                    let mut edge = self.finishes[index].edge;
                    let r = ui.add(egui::Slider::new(&mut edge, EMBOSS_EDGE).fixed_decimals(1).suffix(" mm")).on_hover_text(t!("Width of the slope at the edge. Smaller makes a sharp step, larger a soft rounded rise"));
                    self.finish_slider(r, index, id, "edge", edge);
                    ui.end_row();
                }
            }
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
        if kind == FinishKind::Foil {
            ui.small(t!("The foil takes the shape of the linked artwork and moves with it. Hide the artwork's layer to stamp foil without ink."));
        } else {
            ui.small(t!("The relief takes the shape of the linked artwork and moves with it. Hide the artwork's layer for blind embossing (no ink). Add foil to the same artwork for raised foil."));
        }
        let remove = if kind == FinishKind::Foil { t!("Remove the foil") } else { t!("Remove the embossing") };
        if ui.button(remove).on_hover_text(t!("Ctrl+Z undoes this")).clicked() {
            self.person("finish.delete", json!({ "id": id }));
        }
    }

    /// 加工の数値のつまみ。引いている間は見た目だけ変え、離したときに 1 回の操作として記録する。
    fn finish_slider(&mut self, r: egui::Response, index: usize, id: u32, key: &str, value: f32) {
        let get = |f: &Finish| match key {
            "wave" => f.wave as f32,
            "height" => f.height,
            _ => f.edge,
        };
        let set = |f: &mut Finish, v: f32| match key {
            "wave" => f.wave = v.round().clamp(0.0, 100.0) as u8,
            "height" => f.height = v,
            _ => f.edge = v,
        };
        if r.drag_started() {
            self.slider_before = Some(get(&self.finishes[index]));
        }
        if value != get(&self.finishes[index]) {
            if r.dragged() {
                set(&mut self.finishes[index], value);
                (self.art_dirty, self.back_dirty) = (true, true);
            } else {
                self.person("finish.set", json!({ "id": id, key: value }));
            }
        }
        if r.drag_stopped()
            && let Some(before) = self.slider_before.take()
        {
            let after = get(&self.finishes[index]);
            set(&mut self.finishes[index], before);
            if after != before {
                self.person("finish.set", json!({ "id": id, key: after }));
            }
        }
    }
}
