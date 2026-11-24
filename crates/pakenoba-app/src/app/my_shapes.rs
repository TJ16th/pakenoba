//! 自分の型: 組み込みの型に寸法と紙を決めて、名前を付けて残したもの（どの案件でも使う）。
//!
//! 設定ファイルの `shape` に残す。型の画面の左、組み込みの型の下に並べる。

use pakenoba_core::material::{Coat, Material, Paper};
use pakenoba_core::template::TuckFlaps;
use eframe::egui;
use pakenoba_i18n::t;

use super::settings::ShapePreset;
use super::{App, Dims, SHAPES};

/// いまの型・寸法・紙を、自分の型の形にする。
fn preset_of(name: &str, dims: &Dims, material: &Material) -> ShapePreset {
    ShapePreset {
        name: name.to_owned(),
        sides: dims.sides,
        width: dims.w,
        depth: dims.d,
        height: dims.h,
        thickness: dims.t,
        bulge: dims.bulge,
        roof: dims.roof,
        glue_flap: dims.flaps.glue,
        dust_flap: dims.flaps.dust,
        tuck_flap: dims.flaps.tuck,
        paper: material.paper.key().to_owned(),
        coat: material.coat.key().to_owned(),
    }
}

/// 自分の型から、寸法と紙を戻す。知らない型（新しい版で消えたもの）なら None。
fn dims_of(preset: &ShapePreset) -> Option<(Dims, Material)> {
    if !SHAPES.iter().any(|(s, _)| *s == preset.sides) {
        return None;
    }
    let dims = Dims {
        sides: preset.sides,
        w: preset.width,
        d: preset.depth,
        h: preset.height,
        t: preset.thickness,
        bulge: preset.bulge,
        roof: preset.roof,
        flaps: TuckFlaps { glue: preset.glue_flap, dust: preset.dust_flap, tuck: preset.tuck_flap },
    };
    let material = Material { paper: Paper::from_key(&preset.paper).unwrap_or_default(), coat: Coat::from_key(&preset.coat).unwrap_or_default() };
    Some((dims, material))
}

/// 型の名前と寸法の短い説明（一覧の小さな字）。
fn summary(preset: &ShapePreset) -> String {
    let kind = SHAPES.iter().find(|(s, _)| *s == preset.sides).map_or("?", |(_, name)| pakenoba_i18n::tr(name));
    let size = match preset.sides {
        0 | 2 => format!("{:.0} × {:.0} × {:.0}", preset.width, preset.depth, preset.height),
        _ => format!("{:.0} × {:.0}", preset.width, preset.height),
    };
    format!("{kind}  {size} mm")
}

impl App {
    /// 型の画面の左: 自分の型の一覧と、残すための欄。
    pub(super) fn my_shapes(&mut self, ui: &mut egui::Ui) {
        let current = preset_of("", &self.dims, &self.material);
        let mut apply = None;
        let mut remove = None;
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            if self.settings.shapes.is_empty() {
                ui.weak(t!("Save the size and paper you use often, and pick them here next time."));
            }
            for (index, preset) in self.settings.shapes.iter().enumerate() {
                let same = ShapePreset { name: String::new(), ..preset.clone() } == current;
                let response = ui.selectable_label(same, &preset.name).on_hover_text(summary(preset));
                if response.clicked() && !same {
                    apply = Some(index);
                }
                response.context_menu(|ui| {
                    if ui.button(t!("Delete")).clicked() {
                        remove = Some(index);
                        ui.close();
                    }
                });
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let field = egui::TextEdit::singleline(&mut self.shape_preset_name).hint_text(t!("Name")).desired_width(100.0);
            let entered = ui.add(field).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ready = !self.shape_preset_name.trim().is_empty();
            if (ui.add_enabled(ready, egui::Button::new(t!("Save"))).on_hover_text(t!("Saves this box type, size and paper under the name. It can be used in any project.")).clicked() || entered) && ready {
                let name = std::mem::take(&mut self.shape_preset_name);
                self.save_my_shape(&name);
            }
        });
        if let Some(index) = remove {
            let name = self.settings.shapes.remove(index).name;
            match self.settings.save() {
                Ok(()) => self.say(t!("Deleted \"{name}\" from your box types.")),
                Err(e) => self.say(e),
            }
        }
        if let Some(index) = apply {
            self.use_my_shape(index);
        }
    }

    /// いまの型・寸法・紙を、名前を付けて残す。同じ名前があれば置き換える。
    fn save_my_shape(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let preset = preset_of(name, &self.dims, &self.material);
        match self.settings.shapes.iter_mut().find(|s| s.name == name) {
            Some(old) => *old = preset,
            None => self.settings.shapes.push(preset),
        }
        match self.settings.save() {
            Ok(()) => self.say(t!("Saved \"{name}\" to your box types. It can be used in any project.")),
            Err(e) => self.say(e),
        }
    }

    /// 自分の型にする。型が変わって柄が外れるときは、先に確かめる。
    fn use_my_shape(&mut self, index: usize) {
        let Some((dims, material)) = self.settings.shapes.get(index).and_then(dims_of) else {
            self.say(t!("This box type is not available in this version."));
            return;
        };
        if dims.sides != self.dims.sides && !self.decals.is_empty() {
            self.pending_shape = Some(dims.sides);
            self.pending_dims = Some((dims, material));
            return;
        }
        self.apply_dims(dims, material);
    }

    /// 寸法と紙をまとめて変える（元に戻せる）。型が違えば、型も変える。
    pub(super) fn apply_dims(&mut self, dims: Dims, material: Material) {
        if dims.sides != self.dims.sides {
            self.set_shape(dims.sides);
        } else {
            self.history.record(self.snapshot());
        }
        self.dims = dims;
        self.material = material;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_round_trips_dims_and_paper() {
        let dims = Dims { sides: 0, w: 120.0, d: 40.0, h: 150.0, t: 0.5, bulge: 40.0, roof: 45.0, flaps: TuckFlaps { glue: Some(14.0), dust: None, tuck: Some(16.0) } };
        let material = Material { paper: Paper::Kraft, coat: Coat::MattePp };
        let preset = preset_of("小箱", &dims, &material);
        let text = toml::to_string(&preset).unwrap();
        let back: ShapePreset = toml::from_str(&text).unwrap();
        assert_eq!(back, preset);
        assert_eq!(dims_of(&back), Some((dims, material)));
        // 知らない型は使わない。
        assert_eq!(dims_of(&ShapePreset { sides: 999, ..back }), None);
    }

    #[test]
    fn saved_type_comes_back_and_undoes() {
        let mut app = App::new(&super::super::Initial::default());
        app.dims.w = 123.0;
        app.material.paper = Paper::Kraft;
        app.save_my_shape("  小箱 ");
        assert_eq!(app.settings.shapes.len(), 1);
        assert_eq!(app.settings.shapes[0].name, "小箱");
        // 同じ名前は置き換える。
        app.save_my_shape("小箱");
        assert_eq!(app.settings.shapes.len(), 1);

        app.dims.w = 80.0;
        app.material.paper = Paper::Coated;
        app.use_my_shape(0);
        assert_eq!((app.dims.w, app.material.paper), (123.0, Paper::Kraft));
        app.undo();
        assert_eq!(app.dims.w, 80.0);

        // 型が変わるときは、柄があれば先に確かめる。
        app.settings.shapes[0].sides = 2;
        let has_art = !app.decals.is_empty();
        app.use_my_shape(0);
        if has_art {
            assert_eq!(app.pending_shape, Some(2));
            let (dims, material) = app.pending_dims.take().unwrap();
            app.apply_dims(dims, material);
        }
        assert_eq!((app.dims.sides, app.dims.w), (2, 123.0));
    }
}
