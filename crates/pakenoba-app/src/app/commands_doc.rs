//! コマンドの続き: 案件、型と寸法、素材、柄を置く、文字と図形、書き出し、見た目の確認（PNG）。
//!
//! ファイルを読み書きするコマンドは、人（`person`）以外が呼ぶときは、使う人が決めたフォルダの中だけ。
//! すでにあるファイルには書かない（上書きと削除はさせない。docs/agent-and-mcp.md の 7 章）。

use pakenoba_core::project::{ShapeKind, ShapeSpec, TextAlign, TextSpec};
use pakenoba_core::surface::Decal;
use pakenoba_i18n::t;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use super::{App, PILLOW, PLAIN_PILLOW, Picture, Placed, SHAPES};

/// 「#rrggbb」を色に。
fn color(value: &Value) -> Result<Option<[u8; 3]>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let text = value.as_str().ok_or_else(|| t!("Colors are written like \"#1a2b3c\"").to_owned())?;
    let hex = text.trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(t!("Colors are written like \"#1a2b3c\"").to_owned());
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    Ok(Some([byte(0), byte(2), byte(4)]))
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn number(value: &Value, key: &str) -> Result<Option<f32>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().map(|v| Some(v as f32)).ok_or_else(|| t!("\"{key}\" is a number")),
    }
}

/// 文字の書式を JSON に。
pub(super) fn text_json(spec: &TextSpec) -> Value {
    let align = match spec.align {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
    };
    json!({ "text": spec.text, "font": spec.font, "size": spec.size, "color": hex(spec.color), "align": align, "spacing": spec.spacing, "leading": spec.leading })
}

/// 図形を JSON に。
pub(super) fn shape_json(spec: &ShapeSpec) -> Value {
    json!({ "kind": if spec.kind == ShapeKind::Ellipse { "ellipse" } else { "rect" }, "fill": spec.fill.map(hex), "stroke": spec.stroke.map(hex), "stroke_width": spec.stroke_width, "radius": spec.radius })
}

/// JSON で書いたところだけ、文字の書式を変える。
fn merge_text(mut spec: TextSpec, args: &Value) -> Result<TextSpec, String> {
    if let Some(text) = args.get("text") {
        spec.text = text.as_str().ok_or_else(|| t!("\"text\" is text").to_owned())?.to_owned();
    }
    if let Some(font) = args.get("font") {
        spec.font = font.as_str().ok_or_else(|| t!("\"font\" is a font name").to_owned())?.to_owned();
    }
    if let Some(size) = number(args, "size")? {
        spec.size = size.clamp(0.5, 500.0);
    }
    if let Some(c) = args.get("color") {
        spec.color = color(c)?.unwrap_or(spec.color);
    }
    if let Some(align) = args.get("align") {
        spec.align = match align.as_str() {
            Some("left") => TextAlign::Left,
            Some("center") => TextAlign::Center,
            Some("right") => TextAlign::Right,
            _ => return Err(t!("\"align\" is left, center or right").to_owned()),
        };
    }
    if let Some(spacing) = number(args, "spacing")? {
        spec.spacing = spacing.clamp(-500.0, 2000.0);
    }
    if let Some(leading) = number(args, "leading")? {
        spec.leading = leading.clamp(0.5, 5.0);
    }
    Ok(spec)
}

/// JSON で書いたところだけ、図形を変える。
fn merge_shape(mut spec: ShapeSpec, args: &Value) -> Result<ShapeSpec, String> {
    if let Some(kind) = args.get("kind") {
        spec.kind = match kind.as_str() {
            Some("rect") => ShapeKind::Rect,
            Some("ellipse") => ShapeKind::Ellipse,
            _ => return Err(t!("\"kind\" is rect or ellipse").to_owned()),
        };
    }
    if let Some(c) = args.get("fill") {
        spec.fill = color(c)?;
    }
    if let Some(c) = args.get("stroke") {
        spec.stroke = color(c)?;
    }
    if let Some(w) = number(args, "stroke_width")? {
        spec.stroke_width = w.clamp(0.1, 50.0);
    }
    if let Some(r) = number(args, "radius")? {
        spec.radius = r.clamp(0.0, 500.0);
    }
    Ok(spec)
}

/// 小さく縮めた PNG を、Base64 の文字にする。
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * k)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 絵を、長い辺が `max` 以下になるよう、面積の平均で縮める。
fn shrink(w: usize, h: usize, rgba: &[u8], max: usize) -> (usize, usize, Vec<u8>) {
    let scale = (max as f32 / w.max(h) as f32).min(1.0);
    let (nw, nh) = (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &rgba[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4];
                    for c in 0..4 {
                        sum[c] += u32::from(p[c]);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            for c in 0..4 {
                out[(y * nw + x) * 4 + c] = (sum[c] / n) as u8;
            }
        }
    }
    (nw, nh, out)
}

impl App {
    /// 書き出しの命令の、解像度（`dpi`）と塗り足し（`bleed` mm）。渡されたら、出力の設定を変える。
    fn export_args(&mut self, args: &Value) -> Result<(), String> {
        if let Some(dpi) = args.get("dpi").and_then(Value::as_u64) {
            let dpi = dpi as u32;
            if !super::export::EXPORT_DPI.contains(&dpi) {
                return Err(t!("\"dpi\" must be one of 100, 300, 350, 600").to_owned());
            }
            self.output.dpi = dpi;
        }
        if let Some(bleed) = args.get("bleed").and_then(Value::as_f64) {
            if !(0.0..=5.0).contains(&bleed) {
                return Err(t!("\"bleed\" must be between 0 and 5 mm").to_owned());
            }
            self.output.bleed = bleed as f32;
        }
        Ok(())
    }

    /// 人以外がファイルを扱ってよいフォルダ（設定と、起動のときに渡したもの）。
    fn allowed_folders(&self) -> Vec<PathBuf> {
        self.settings.agent_folders.iter().chain(&self.allowed).filter_map(|p| std::fs::canonicalize(p).ok()).collect()
    }

    /// ファイルの場所を確かめる。人以外は、決めたフォルダの中だけ。書くときは、すでにあるファイルには書かない。
    fn checked_path(&self, args: &Value, write: bool) -> Result<PathBuf, String> {
        let raw = args.get("path").and_then(Value::as_str).ok_or_else(|| t!("\"path\" (a file path) is missing").to_owned())?;
        let path = PathBuf::from(raw);
        if self.actor == "person" {
            return Ok(path);
        }
        if !path.is_absolute() {
            return Err(t!("Give the full path of the file").to_owned());
        }
        let folders = self.allowed_folders();
        if folders.is_empty() {
            return Err(t!("No folders are allowed for AI yet. The user can allow folders in Edit > Preferences > Folders AI may use (or with --allow when starting the MCP server).").to_owned());
        }
        let probe = if write { path.parent().map(Path::to_path_buf).unwrap_or_default() } else { path.clone() };
        let real = std::fs::canonicalize(&probe).map_err(|e| t!("Cannot find {}: {e}", probe.display()))?;
        if !folders.iter().any(|f| real.starts_with(f)) {
            return Err(t!("{} is outside the folders allowed for AI", path.display()));
        }
        if write && path.exists() {
            return Err(t!("{} already exists. AI does not overwrite files; choose another name.", path.display()));
        }
        Ok(path)
    }

    pub(super) fn dispatch_doc(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        if let Some(result) = self.dispatch_finish(name, args) {
            return result;
        }
        match name {
            "project.info" => Ok(json!({
                "name": self.project_name(),
                "path": self.file.path.as_ref().map(|p| p.display().to_string()),
                "unsaved": self.unsaved(),
                "shape": self.shape_summary(),
                "layers": self.decals.len(),
                "assets": self.assets.iter().filter(|a| !a.bundled).count(),
                "allowed_folders": self.allowed_folders().iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            })),
            "project.new" => {
                if !self.pristine() && !self.open_tab() {
                    return Err(t!("Could not open a new tab").to_owned());
                }
                self.apply_project(pakenoba_core::project::Project { shape: super::Dims::for_shape(0, 0.5).shape(), material: Default::default(), finishes: Vec::new(), assets: Vec::new(), decals: Vec::new(), base: None, back_base: None, log: Vec::new(), versions: Vec::new() });
                self.file.path = None;
                self.mark_saved();
                Ok(Value::Null)
            }
            "project.open" => {
                let path = self.checked_path(args, false)?;
                let opened = pakenoba_core::project::Project::open(&path).map_err(|e| e.to_string())?;
                if !self.pristine() && !self.open_tab() {
                    return Err(t!("Could not open a new tab").to_owned());
                }
                let problems = self.apply_project(opened.project);
                self.file.path = Some(path);
                self.mark_saved();
                let all: Vec<String> = opened.problems.into_iter().chain(problems).collect();
                Ok(json!({ "problems": all }))
            }
            "project.save_as" => {
                let path = self.checked_path(args, true)?;
                let path = if path.extension().is_some() { path } else { path.with_extension(pakenoba_core::project::EXTENSION) };
                let project = self.project()?;
                project.save(&path).map_err(|e| e.to_string())?;
                if self.actor == "person" || self.file.path.is_none() {
                    self.file.path = Some(path.clone());
                    self.mark_saved();
                }
                Ok(json!({ "path": path.display().to_string() }))
            }
            "material.set" => {
                use pakenoba_core::material::{Coat, Paper};
                let mut material = self.material;
                if let Some(key) = args.get("paper").and_then(Value::as_str) {
                    material.paper = Paper::from_key(key).ok_or_else(|| t!("Unknown paper \"{key}\""))?;
                }
                if let Some(key) = args.get("coat").and_then(Value::as_str) {
                    material.coat = Coat::from_key(key).ok_or_else(|| t!("Unknown coating \"{key}\""))?;
                }
                self.material = material;
                Ok(json!({ "paper": material.paper.key(), "coat": material.coat.key() }))
            }
            "shape.set" => {
                let kind = args.get("kind").and_then(Value::as_str).ok_or_else(|| t!("\"kind\" is missing").to_owned())?;
                let sides = match kind {
                    "tuck_box" => 0,
                    "gable" => 2,
                    "pillow" => PILLOW,
                    "plain_pillow" => PLAIN_PILLOW,
                    "prism" => {
                        let n = args.get("sides").and_then(Value::as_u64).unwrap_or(6) as usize;
                        if !SHAPES.iter().any(|(s, _)| *s == n) {
                            return Err(t!("Prisms have 3, 5, 6 or 8 sides").to_owned());
                        }
                        n
                    }
                    "ecma" => super::ECMA,
                    other => return Err(t!("Unknown box type \"{other}\" (tuck_box, gable, pillow, plain_pillow, prism, ecma)")),
                };
                if sides == super::ECMA {
                    // ECMA の型: 番号（「A20.20.03.01」）で選ぶ。寸法は、ECMA の型どうしなら、そのまま。
                    let text = args.get("code").and_then(Value::as_str).ok_or_else(|| t!("\"code\" is missing (an ECMA number such as A20.20.03.01)").to_owned())?;
                    let code = pakenoba_core::ecma::Code::parse(text).ok_or_else(|| t!("\"{text}\" is not an ECMA number that PAKENOBA can make (class A: A20.20.03.01 and so on)."))?;
                    let next = if self.dims.sides == super::ECMA { super::Dims { ecma: code.to_u32(), ..self.dims } } else { super::Dims { ecma: code.to_u32(), ..super::Dims::for_shape(super::ECMA, self.dims.t) } };
                    let removed = if next.same_layout(&self.dims) { 0 } else { self.decals.len() };
                    self.apply_dims(next, self.material);
                    self.rebuild_if_needed();
                    return Ok(json!({ "removed_layers": removed, "code": code.name(), "listed": code.listed(), "panels": self.template.panels.iter().map(|p| p.name).collect::<Vec<_>>() }));
                }
                let removed = if sides == self.dims.sides { 0 } else { self.decals.len() };
                self.set_shape(sides);
                self.rebuild_if_needed();
                Ok(json!({ "removed_layers": removed, "panels": self.template.panels.iter().map(|p| p.name).collect::<Vec<_>>() }))
            }
            "shape.dimensions" => {
                let d = &mut self.dims;
                let set = |value: &mut f32, key: &str, range: std::ops::RangeInclusive<f32>| -> Result<(), String> {
                    if let Some(v) = number(args, key)? {
                        *value = v.clamp(*range.start(), *range.end());
                    }
                    Ok(())
                };
                set(&mut d.w, "width", 40.0..=300.0)?;
                set(&mut d.d, "depth", 20.0..=200.0)?;
                set(&mut d.h, "height", 40.0..=300.0)?;
                set(&mut d.t, "thickness", 0.3..=6.0)?;
                set(&mut d.bulge, "bulge", 15.0..=90.0)?;
                set(&mut d.roof, "roof", 10.0..=150.0)?;
                for (key, flap) in [("glue_flap", &mut d.flaps.glue), ("dust_flap", &mut d.flaps.dust), ("tuck_flap", &mut d.flaps.tuck), ("handle_bottom", &mut d.handle.bottom), ("handle_top", &mut d.handle.top), ("handle_height", &mut d.handle.height)] {
                    match args.get(key) {
                        None => {}
                        Some(Value::String(s)) if s == "auto" => *flap = None,
                        Some(v) => *flap = Some(v.as_f64().ok_or_else(|| t!("\"{key}\" is a number or \"auto\""))? as f32),
                    }
                }
                self.rebuild_if_needed();
                let d = self.dims;
                Ok(json!({ "width": d.w, "depth": d.d, "height": d.h, "thickness": d.t, "summary": self.shape_summary(), "problems": self.problems.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>() }))
            }
            "asset.list" => Ok(Value::Array(
                self.assets
                    .iter()
                    .enumerate()
                    .map(|(i, a)| json!({ "index": i, "name": a.name, "size": a.size, "bundled": a.bundled, "kind": match a.picture { Picture::Logo => "logo", Picture::Band => "band", Picture::Image { vector: true, .. } => "vector_image", _ => "image" } }))
                    .collect(),
            )),
            "asset.import" => {
                let path = self.checked_path(args, false)?;
                let index = self.import_image(&path, true).ok_or_else(|| t!("Could not import {}", path.display()))?;
                Ok(json!({ "index": index, "name": self.assets[index].name }))
            }
            "decal.add" => self.add_by_command(args),
            "text.set" => {
                let index = self.index_of(super::commands::int(args, "id")?)?;
                let Picture::Text(art) = &self.decals[index].picture else { return Err(t!("Layer {} is not text", self.decals[index].layer.id)) };
                let spec = merge_text(art.spec.clone(), args)?;
                self.checkpoint();
                self.retype(index, spec);
                Ok(self.layer_json(index))
            }
            "shape_art.set" => {
                let index = self.index_of(super::commands::int(args, "id")?)?;
                let Picture::Shape(old) = &self.decals[index].picture else { return Err(t!("Layer {} is not a shape", self.decals[index].layer.id)) };
                let spec = merge_shape(old.clone(), args)?;
                self.checkpoint();
                self.reshape(index, spec);
                Ok(self.layer_json(index))
            }
            "export.psd" => {
                let path = self.checked_path(args, true)?;
                self.export_args(args)?;
                if self.actor != "person" && self.has_back_decals() && super::export::back_path(&path).exists() {
                    return Err(t!("{} already exists. AI does not overwrite files; choose another name.", super::export::back_path(&path).display()));
                }
                if self.actor != "person" && !self.finishes.is_empty() && super::export::notes_path(&path).exists() {
                    return Err(t!("{} already exists. AI does not overwrite files; choose another name.", super::export::notes_path(&path).display()));
                }
                let files = self.with_sharp_text(|app| app.psd_files(&path))?;
                Ok(json!({ "files": files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>() }))
            }
            "export.pdf" => {
                let path = self.checked_path(args, true)?;
                let pages = self.print_file(&path)?;
                Ok(json!({ "path": path.display().to_string(), "sheets": pages }))
            }
            "export.print_ready" => {
                let path = self.checked_path(args, true)?;
                self.export_args(args)?;
                let pages = self.with_sharp_text(|app| app.ready_file(&path))?;
                let coarse: Vec<Value> = self.coarse_images().into_iter().map(|(name, dpi)| json!({ "layer": name, "dpi": dpi })).collect();
                Ok(json!({ "path": path.display().to_string(), "pages": pages, "dpi": self.export_frame().dpi_value().round(), "coarse_images": coarse }))
            }
            "version.list" => Ok(Value::Array(self.versions.iter().map(|v| json!({ "name": v.name, "time": v.time, "auto": v.auto, "layers": v.project.decals.len() })).collect())),
            "version.keep" => {
                let name = args.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| t!("\"name\" is missing").to_owned())?.to_owned();
                if !self.keep_version(&name, false) {
                    return Err(t!("Could not keep a version").to_owned());
                }
                Ok(json!({ "versions": self.versions.len() }))
            }
            "render.preview" => {
                let back = match args.get("side").and_then(Value::as_str).unwrap_or("front") {
                    "front" => false,
                    "back" => true,
                    other => return Err(t!("\"side\" is front or back, not \"{other}\"")),
                };
                let max = args.get("max").and_then(Value::as_u64).unwrap_or(1024).clamp(128, 4096) as usize;
                self.rebuild_if_needed();
                let mut art = self.painted(back);
                crate::art::alpha_over(&mut art, &self.sheet.guides);
                let (w, h, small) = shrink(self.sheet.w, self.sheet.h, &art, max);
                let png = super::outputs::png_bytes(w as u32, h as u32, &small)?;
                let mm = [self.template.max[0] - self.template.min[0], self.template.max[1] - self.template.min[1]];
                Ok(json!({ "side": if back { "back" } else { "front" }, "width": w, "height": h, "mm": mm, "png_base64": base64(&png), "note": "The flat dieline with artwork and guides (cut lines, fold lines, glue flaps). The back is seen from the back (flipped left to right)." }))
            }
            _ => Err(t!("Unknown command \"{name}\"")),
        }
    }

    /// `decal.add`: 素材、付属の見本、文字、図形のどれかを置く。
    fn add_by_command(&mut self, args: &Value) -> Result<Value, String> {
        self.rebuild_if_needed();
        let given = ["asset", "builtin", "text", "shape"].iter().filter(|k| args.get(**k).is_some()).count();
        if given != 1 {
            return Err(t!("Give exactly one of \"asset\", \"builtin\", \"text\" and \"shape\"").to_owned());
        }
        let (picture, size) = if let Some(index) = args.get("asset") {
            let index = index.as_u64().map(|i| i as usize).filter(|&i| i < self.assets.len()).ok_or_else(|| t!("There is no asset with that index (see asset.list)").to_owned())?;
            (self.assets[index].picture.clone(), self.assets[index].size)
        } else if let Some(name) = args.get("builtin") {
            match name.as_str() {
                Some("logo") => (Picture::Logo, [46.0, 46.0]),
                Some("band") => (Picture::Band, [120.0, 14.0]),
                _ => return Err(t!("\"builtin\" is logo or band").to_owned()),
            }
        } else if let Some(text) = args.get("text") {
            let mut base = super::typing::default_style();
            base.font = self.fonts.default_family().unwrap_or_default();
            let spec = merge_text(base, text)?;
            if spec.text.trim().is_empty() {
                return Err(t!("The text is empty").to_owned());
            }
            let picture = self.text_picture(&spec)?;
            let Picture::Text(art) = &picture else { unreachable!() };
            let size = art.size_mm;
            (picture, size)
        } else {
            let spec = merge_shape(super::shapes::default_shape(), &args["shape"])?;
            (Picture::Shape(spec), [40.0, 30.0])
        };
        let panel = match args.get("panel") {
            Some(p) => {
                let p = p.as_str().ok_or_else(|| t!("\"panel\" is a panel name").to_owned())?;
                self.template.panels.iter().position(|q| q.name == p).ok_or_else(|| t!("There is no panel named \"{p}\""))?
            }
            None => self.template.panels.iter().position(|p| p.name == "正面").unwrap_or(0),
        };
        let mut decal = Decal { panel, at: [0.5, 0.5], size, angle: 0.0 };
        if let Some(at) = args.get("at") {
            decal.at = super::commands::pair(at).ok_or_else(|| t!("\"at\" is two numbers").to_owned())?;
        }
        if let Some(size) = args.get("size") {
            decal.size = super::commands::pair(size).filter(|s| s[0] > 0.0 && s[1] > 0.0).ok_or_else(|| t!("\"size\" is two numbers above 0 (mm)").to_owned())?;
        }
        if let Some(angle) = number(args, "angle")? {
            decal.angle = angle.to_radians();
        }
        let back = args.get("back").and_then(Value::as_bool).unwrap_or(false);
        self.checkpoint();
        self.decals.push(Placed { decal, picture, back, layer: Default::default() });
        let index = self.decals.len() - 1;
        self.select_only(Some(index));
        self.ensure_ids();
        Ok(self.layer_json(index))
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    #[test]
    fn base64は標準の書き方() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn 型と寸法を変え_柄を置いて_見た目をpngで返す() {
        let mut app = App::new(&Initial::default());
        app.decals.clear();
        app.run_command("agent:test", "shape.set", json!({ "kind": "prism", "sides": 6 })).unwrap();
        assert_eq!(app.dims.sides, 6);
        app.run_command("agent:test", "shape.dimensions", json!({ "width": 70, "height": 160 })).unwrap();
        assert_eq!((app.dims.w, app.dims.h), (70.0, 160.0));
        let panel = app.template.panels[0].name;
        let shape = app.run_command("agent:test", "decal.add", json!({ "shape": { "kind": "ellipse", "fill": "#ff0000" }, "panel": panel, "size": [20, 20] })).unwrap();
        let id = shape["id"].as_i64().unwrap();
        app.run_command("agent:test", "shape_art.set", json!({ "id": id, "fill": "#00ff00", "stroke": null })).unwrap();
        let Picture::Shape(spec) = &app.decals[0].picture else { panic!() };
        assert_eq!(spec.fill, Some([0, 255, 0]));
        let logo = app.run_command("agent:test", "decal.add", json!({ "builtin": "logo", "back": true })).unwrap();
        assert_eq!(logo["side"], "back");
        assert!(app.run_command("agent:test", "decal.add", json!({ "builtin": "logo", "shape": {} })).is_err());
        let preview = app.run_command("agent:test", "render.preview", json!({ "max": 256 })).unwrap();
        assert!(preview["width"].as_u64().unwrap() <= 256 && preview["png_base64"].as_str().unwrap().starts_with("iVBOR"));
        // 柄を置く、図形を変える、が記録に残る。
        assert!(app.log.iter().any(|e| e.command == "decal.add") && app.log.iter().any(|e| e.command == "shape_art.set"));
    }

    #[test]
    fn ecmaの番号で型を変える() {
        let mut app = App::new(&Initial::default());
        app.decals.clear();
        let out = app.run_command("agent:test", "shape.set", json!({ "kind": "ecma", "code": "A55.20.03.01" })).unwrap();
        assert_eq!(out["code"], "A55.20.03.01");
        assert_eq!(app.dims.ecma, 55200301);
        assert!(app.template.panels.iter().any(|p| p.name == "上の差し込み"));
        app.run_command("agent:test", "shape.dimensions", json!({ "width": 120 })).unwrap();
        // ECMA どうしで番号を変えても、寸法はそのまま。
        app.run_command("agent:test", "shape.set", json!({ "kind": "ecma", "code": "a20200101" })).unwrap();
        assert_eq!((app.dims.ecma, app.dims.w), (20200101, 120.0));
        assert!(app.run_command("agent:test", "shape.set", json!({ "kind": "ecma", "code": "A99.20.03.01" })).is_err());
        assert!(app.run_command("agent:test", "shape.set", json!({ "kind": "ecma" })).is_err());
        assert_eq!(app.inspect()["shape"]["ecma"], "A20.20.01.01");
    }

    #[test]
    fn 人以外は_決めたフォルダの中にだけ書け_上書きはしない() {
        let dir = std::env::temp_dir().join(format!("pakenoba-agent-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ok")).unwrap();
        let mut app = App::new(&Initial::default());
        let target = dir.join("ok").join("box.psd");
        // フォルダを決めていなければ、書かない。
        assert!(app.run_command("agent:test", "export.psd", json!({ "path": target })).is_err());
        app.allowed.push(dir.join("ok"));
        app.run_command("agent:test", "export.psd", json!({ "path": target })).unwrap();
        assert!(target.exists());
        // 2 回目は、すでにあるので書かない。
        assert!(app.run_command("agent:test", "export.psd", json!({ "path": target })).is_err());
        // 決めたフォルダの外には書かない。
        assert!(app.run_command("agent:test", "export.pdf", json!({ "path": dir.join("out.pdf") })).is_err());
        // 名前を付けて保存して、開ける。
        let saved = dir.join("ok").join("box.pknb");
        app.run_command("agent:test", "project.save_as", json!({ "path": saved })).unwrap();
        let mut other = App::new(&Initial::default());
        other.allowed.push(dir.join("ok"));
        other.run_command("agent:test", "project.open", json!({ "path": saved })).unwrap();
        assert_eq!(other.decals.len(), app.decals.len());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
