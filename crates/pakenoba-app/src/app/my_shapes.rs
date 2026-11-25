//! 自分の型: 組み込みの型に寸法と紙を決めて、名前を付けて残したもの（どの案件でも使う）。
//!
//! 設定ファイルの `shape` に残す。型の画面の左、組み込みの型の下に並べる。

use pakenoba_core::material::{Coat, Material, Paper};
use pakenoba_core::template::TuckFlaps;
use eframe::egui;
use pakenoba_i18n::t;

use super::settings::{DielinePreset, ShapePreset};
use super::{App, Dims, ECMA, IMPORTED, SHAPES};
use pakenoba_core::dieline;

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
        dieline: (dims.sides == IMPORTED).then(|| dieline_preset(dims.dieline)).flatten(),
        ecma: (dims.sides == ECMA).then(|| pakenoba_core::ecma::Code::from_u32(dims.ecma).map(|c| c.name())).flatten(),
    }
}

/// 読み込んだ型の、ファイルの名前と読み方。
fn dieline_preset(key: u64) -> Option<DielinePreset> {
    use sha2::{Digest, Sha256};
    let src = dieline::source(key)?;
    let id: String = Sha256::digest(src.bytes.as_slice()).iter().take(16).map(|b| format!("{b:02x}")).collect();
    Some(DielinePreset {
        name: src.name.clone(),
        file: format!("dielines/{id}.dxf"),
        unit_mm: src.settings.unit_mm,
        mirror: src.settings.mirror,
        lines: src.drawing.groups.iter().zip(&src.settings.kinds).map(|(g, k)| (g.name.clone(), k.key().to_owned())).collect(),
        folds: src.settings.folds.iter().map(|(p, a)| (p.to_string(), *a)).collect(),
    })
}

/// 残した型の線のファイルを読み、読み方をそろえて、番号を返す。
fn load_dieline(preset: &DielinePreset, folder: &std::path::Path) -> Result<u64, String> {
    let bytes = std::fs::read(folder.join(&preset.file)).map_err(|e| t!("Could not read the dieline file of this box type: {e}"))?;
    let bytes = std::sync::Arc::new(bytes);
    let drawing = dieline::read_dxf(&bytes)?;
    let mut settings = pakenoba_core::template::ImportSettings::guess(&drawing);
    settings.unit_mm = preset.unit_mm;
    settings.mirror = preset.mirror;
    for (group, kind) in drawing.groups.iter().zip(settings.kinds.iter_mut()) {
        if let Some(k) = preset.lines.get(&group.name).and_then(|k| dieline::LineKind::from_key(k)) {
            *kind = k;
        }
    }
    settings.folds = preset.folds.iter().filter_map(|(p, a)| Some((p.parse().ok()?, *a))).collect();
    dieline::register(&preset.name, bytes, Some(settings))
}

/// 自分の型から、寸法と紙を戻す。知らない型（新しい版で消えたもの）や、読み込んだ型のファイルがなければ None。
fn dims_of(preset: &ShapePreset, folder: Option<&std::path::Path>) -> Option<(Dims, Material)> {
    let dieline = match (&preset.dieline, folder) {
        (Some(d), Some(folder)) if preset.sides == IMPORTED => load_dieline(d, folder).ok()?,
        (None, _) if SHAPES.iter().any(|(s, _)| *s == preset.sides) || preset.sides == ECMA => 0,
        _ => return None,
    };
    let ecma = match preset.sides {
        ECMA => pakenoba_core::ecma::Code::parse(preset.ecma.as_deref()?)?.to_u32(),
        _ => 0,
    };
    let dims = Dims {
        sides: preset.sides,
        w: preset.width,
        d: preset.depth,
        h: preset.height,
        t: preset.thickness,
        bulge: preset.bulge,
        roof: preset.roof,
        flaps: TuckFlaps { glue: preset.glue_flap, dust: preset.dust_flap, tuck: preset.tuck_flap },
        dieline,
        ecma,
    };
    let material = Material { paper: Paper::from_key(&preset.paper).unwrap_or_default(), coat: Coat::from_key(&preset.coat).unwrap_or_default() };
    Some((dims, material))
}

/// 型の名前と寸法の短い説明（一覧の小さな字）。
fn summary(preset: &ShapePreset) -> String {
    if let Some(d) = &preset.dieline {
        return format!("{} (DXF)", d.name);
    }
    if let Some(code) = &preset.ecma {
        return format!("ECMA {code}  {:.0} × {:.0} × {:.0} mm", preset.width, preset.depth, preset.height);
    }
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
        // 読み込んだ型は、ファイルの写しを設定のフォルダに置く（元のファイルを動かしても使えるように）。
        if let Some(d) = &preset.dieline {
            let (Some(folder), Some(src)) = (self.settings.folder(), dieline::source(self.dims.dieline)) else {
                return self.say(t!("This box type cannot be kept because the settings cannot be saved."));
            };
            let path = folder.join(&d.file);
            if !path.exists() {
                let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, src.bytes.as_slice()));
                if let Err(e) = written {
                    return self.say(t!("Could not keep the dieline file: {e}"));
                }
            }
        }
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
        let folder = self.settings.folder();
        let Some((dims, material)) = self.settings.shapes.get(index).and_then(|p| dims_of(p, folder.as_deref())) else {
            self.say(t!("This box type is not available in this version."));
            return;
        };
        if !dims.same_layout(&self.dims) && !self.decals.is_empty() {
            self.pending_shape = Some(dims.sides);
            self.pending_dims = Some((dims, material));
            return;
        }
        self.apply_dims(dims, material);
    }

    /// 寸法と紙をまとめて変える（元に戻せる）。型が違えば、型も変える。
    pub(super) fn apply_dims(&mut self, dims: Dims, material: Material) {
        if dims.same_layout(&self.dims) {
            self.history.record(self.snapshot());
        } else {
            // 型が変わる（パネルの並びが変わる）。置いた柄は外れるが、元に戻せる。
            self.checkpoint();
            let removed = self.decals.len();
            self.fold = 1.0;
            self.open = [0.0; 2];
            (self.animate_to, self.open_to) = (None, [None; 2]);
            self.fold_pick = None;
            if removed > 0 {
                self.say(t!("Changing the box type removed {removed} placed artwork(s). Press Ctrl+Z to get the previous box type and artwork back."));
            }
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
        let dims = Dims { sides: 0, w: 120.0, d: 40.0, h: 150.0, t: 0.5, bulge: 40.0, roof: 45.0, flaps: TuckFlaps { glue: Some(14.0), dust: None, tuck: Some(16.0) }, dieline: 0, ecma: 0 };
        let material = Material { paper: Paper::Kraft, coat: Coat::MattePp };
        let preset = preset_of("小箱", &dims, &material);
        let text = toml::to_string(&preset).unwrap();
        let back: ShapePreset = toml::from_str(&text).unwrap();
        assert_eq!(back, preset);
        assert_eq!(dims_of(&back, None), Some((dims, material)));
        // 知らない型は使わない。
        assert_eq!(dims_of(&ShapePreset { sides: 999, ..back }, None), None);
    }

    #[test]
    fn ecma_preset_keeps_its_number() {
        let dims = Dims { ecma: 55200301, w: 90.0, ..Dims::for_shape(ECMA, 0.4) };
        let preset = preset_of("組み底", &dims, &Material::default());
        assert_eq!(preset.ecma.as_deref(), Some("A55.20.03.01"));
        let back: ShapePreset = toml::from_str(&toml::to_string(&preset).unwrap()).unwrap();
        assert_eq!(dims_of(&back, None).map(|(d, _)| d), Some(dims));
        // 番号が読めなければ使わない。
        assert_eq!(dims_of(&ShapePreset { ecma: Some("X".into()), ..back }, None), None);
    }

    fn sample_dxf() -> std::sync::Arc<Vec<u8>> {
        let text = "0\nSECTION\n2\nENTITIES\n0\nLWPOLYLINE\n8\nKNIFE\n70\n1\n10\n0\n20\n0\n10\n200\n20\n0\n10\n200\n20\n100\n10\n0\n20\n100\n\
                    0\nLINE\n8\nSCORE\n10\n100\n20\n0\n11\n100\n21\n100\n0\nENDSEC\n0\nEOF\n";
        std::sync::Arc::new(text.as_bytes().to_vec())
    }

    #[test]
    fn imported_type_is_kept_with_its_file_and_folds() {
        let bytes = sample_dxf();
        let drawing = dieline::read_dxf(&bytes).unwrap();
        let mut settings = pakenoba_core::template::ImportSettings::guess(&drawing);
        settings.folds.insert(1, 60.0);
        let key = dieline::register("tray.dxf", bytes.clone(), Some(settings)).unwrap();
        let dims = Dims { sides: IMPORTED, dieline: key, ..Dims::for_shape(0, 0.4) };
        let material = Material::default();
        let preset = preset_of("トレー", &dims, &material);
        let d = preset.dieline.clone().unwrap();
        // 設定のフォルダに写しを置いたことにする。
        let folder = std::env::temp_dir().join(format!("pkn-shapes-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("dielines")).unwrap();
        std::fs::write(folder.join(&d.file), bytes.as_slice()).unwrap();
        let text = toml::to_string(&preset).unwrap();
        let back: ShapePreset = toml::from_str(&text).unwrap();
        let (again, _) = dims_of(&back, Some(&folder)).unwrap();
        assert!(again.same_layout(&dims));
        assert_eq!(dieline::source(again.dieline).unwrap().settings.folds.get(&1), Some(&60.0));
        // ファイルがなければ使えない。
        assert!(dims_of(&back, None).is_none());
        let _ = std::fs::remove_dir_all(&folder);
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
