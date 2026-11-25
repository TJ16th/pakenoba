//! コマンド: 画面、MCP、エージェントが同じ操作を呼ぶ入口。
//!
//! - コマンドは、名前（`layer.blend.set` など）、引数（JSON）、結果（JSON）を持つ。
//! - レイヤーは番号ではなく、レイヤーごとの決まった印（`id`）で指す。並べ替えても変わらない。
//! - 1 回のコマンドが、元に戻す 1 回ぶん。`run_batch` でまとめて呼んだものは、まとめて 1 回ぶん。
//! - 変えるコマンドは、操作の記録（`log`）に 1 件ずつ残す。続けて同じものを変えた記録（スライダーのドラッグなど）は、1 件にまとめる。
//!
//! 詳しくは docs/agent-and-mcp.md。

use pakenoba_core::project::{self, Adjustment, Blend, LogEntry, adjustment_from_json, adjustment_to_json};
use pakenoba_i18n::t;
use serde_json::{Value, json};

use super::layers::MaskFrom;
use super::{App, Picture};

/// コマンド 1 つの説明（一覧に出す）。
pub(crate) struct CommandInfo {
    pub name: &'static str,
    /// 何をするか（英語。AI に渡す。MCP とエージェントの欄で使う）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub summary: &'static str,
    /// 引数の形（JSON Schema。MCP とエージェントの欄で使う）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub args: &'static str,
    /// 案件を変えない（記録にも残さない）。
    pub read_only: bool,
}

macro_rules! ids {
    () => {
        r#"{"type":"array","items":{"type":"integer"},"description":"layer ids"}"#
    };
}

macro_rules! schema {
    ($($key:literal : $val:expr),* ; required: [$($req:literal),*]) => {
        concat!(r#"{"type":"object","properties":{"#, $( "\"", $key, "\":", $val, "," ,)* r#""_":{}},"required":["#, $( "\"", $req, "\",", )* r#""_"]}"#)
    };
}

/// コマンドの一覧。並びは、まとまりごと。
pub(crate) const COMMANDS: &[CommandInfo] = &[
    CommandInfo { name: "doc.inspect", summary: "Describe the project: box shape and size, panels, layers (id, kind, name, side, depth, visibility, locks, opacity, fill, blend mode, clipping, adjustments, mask, position) and the selection.", args: schema!(; required: []), read_only: true },
    CommandInfo { name: "layer.list", summary: "List the layers from bottom to top.", args: schema!(; required: []), read_only: true },
    CommandInfo { name: "layer.select", summary: "Select layers (the first id becomes the main selection). An empty list clears the selection.", args: schema!("ids": ids!(); required: ["ids"]), read_only: true },
    CommandInfo { name: "layer.rename", summary: "Rename a layer. An empty name goes back to the automatic name.", args: schema!("id": r#"{"type":"integer"}"#, "name": r#"{"type":"string"}"#; required: ["id", "name"]), read_only: false },
    CommandInfo { name: "layer.visible", summary: "Show or hide layers.", args: schema!("ids": ids!(), "visible": r#"{"type":"boolean"}"#; required: ["ids", "visible"]), read_only: false },
    CommandInfo { name: "layer.lock", summary: "Lock or unlock layers. all: cannot be grabbed, changed or deleted. pixels: adjustments and mask cannot change. position: cannot move, resize or rotate.", args: schema!("ids": ids!(), "all": r#"{"type":"boolean"}"#, "pixels": r#"{"type":"boolean"}"#, "position": r#"{"type":"boolean"}"#; required: ["ids"]), read_only: false },
    CommandInfo { name: "layer.blend.set", summary: "Set the blend mode of layers (normal, dissolve is not supported, darken, multiply, color_burn, linear_burn, darker_color, lighten, screen, color_dodge, linear_dodge, lighter_color, overlay, soft_light, hard_light, vivid_light, linear_light, pin_light, hard_mix, difference, exclusion, subtract, divide, hue, saturation, color, luminosity; pass_through for groups).", args: schema!("ids": ids!(), "blend": r#"{"type":"string"}"#; required: ["ids", "blend"]), read_only: false },
    CommandInfo { name: "layer.opacity.set", summary: "Set the opacity of layers (0 to 100).", args: schema!("ids": ids!(), "opacity": r#"{"type":"integer","minimum":0,"maximum":100}"#; required: ["ids", "opacity"]), read_only: false },
    CommandInfo { name: "layer.fill.set", summary: "Set the fill (opacity of the layer's own content, 0 to 100). Groups have no fill.", args: schema!("ids": ids!(), "fill": r#"{"type":"integer","minimum":0,"maximum":100}"#; required: ["ids", "fill"]), read_only: false },
    CommandInfo { name: "layer.reorder", summary: "Move layers up (step 1) or down (step -1) in the stack. They go in and out of groups at the group edges, like Photoshop.", args: schema!("ids": ids!(), "step": r#"{"type":"integer","enum":[1,-1]}"#; required: ["ids", "step"]), read_only: false },
    CommandInfo { name: "layer.clip", summary: "Make a layer a clipping mask of the layer just below it (or release it).", args: schema!("id": r#"{"type":"integer"}"#, "clipped": r#"{"type":"boolean"}"#; required: ["id", "clipped"]), read_only: false },
    CommandInfo { name: "group.create", summary: "Put layers into a new group. Returns the id of the group.", args: schema!("ids": ids!(); required: ["ids"]), read_only: false },
    CommandInfo { name: "group.ungroup", summary: "Remove a group and keep its contents.", args: schema!("id": r#"{"type":"integer"}"#; required: ["id"]), read_only: false },
    CommandInfo { name: "decal.delete", summary: "Delete layers (a group is deleted with its contents). Locked layers are kept.", args: schema!("ids": ids!(); required: ["ids"]), read_only: false },
    CommandInfo { name: "decal.transform", summary: "Move, resize or rotate artwork. panel: panel name. at: center inside the panel's bounding box (0..1, origin bottom left). size: width and height in mm. angle: degrees, counterclockwise. back: true to place it on the back of the sheet (inside of the box). Omitted values stay as they are.", args: schema!("id": r#"{"type":"integer"}"#, "panel": r#"{"type":"string"}"#, "at": r#"{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}"#, "size": r#"{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}"#, "angle": r#"{"type":"number"}"#, "back": r#"{"type":"boolean"}"#; required: ["id"]), read_only: false },
    CommandInfo { name: "adjust.add", summary: "Add an adjustment that affects only this artwork. adjustment: {kind, ...values} as in doc.inspect (kinds: brightness_contrast, levels, curves, exposure, vibrance, hue_saturation, color_balance, black_white, photo_filter, invert, posterize, threshold, gradient_map, subject, clear_background, flatten_shading, reduce_colors).", args: schema!("id": r#"{"type":"integer"}"#, "adjustment": r#"{"type":"object"}"#; required: ["id", "adjustment"]), read_only: false },
    CommandInfo { name: "adjust.set", summary: "Replace the values of an adjustment (index counts from the first one applied).", args: schema!("id": r#"{"type":"integer"}"#, "index": r#"{"type":"integer"}"#, "adjustment": r#"{"type":"object"}"#; required: ["id", "index", "adjustment"]), read_only: false },
    CommandInfo { name: "adjust.toggle", summary: "Turn an adjustment on or off.", args: schema!("id": r#"{"type":"integer"}"#, "index": r#"{"type":"integer"}"#, "on": r#"{"type":"boolean"}"#; required: ["id", "index", "on"]), read_only: false },
    CommandInfo { name: "adjust.move", summary: "Move an adjustment to another place in the order (to: the new index).", args: schema!("id": r#"{"type":"integer"}"#, "index": r#"{"type":"integer"}"#, "to": r#"{"type":"integer"}"#; required: ["id", "index", "to"]), read_only: false },
    CommandInfo { name: "adjust.remove", summary: "Remove an adjustment.", args: schema!("id": r#"{"type":"integer"}"#, "index": r#"{"type":"integer"}"#; required: ["id", "index"]), read_only: false },
    CommandInfo { name: "adjustment_layer.add", summary: "Add an adjustment layer that affects everything below it (inside a group that is not pass through: only the group). It goes just above the layer `above` (or at the top of the front). Returns its id.", args: schema!("adjustment": r#"{"type":"object"}"#, "above": r#"{"type":"integer"}"#; required: ["adjustment"]), read_only: false },
    CommandInfo { name: "mask.add", summary: "Add a layer mask. from: reveal_all, transparency.", args: schema!("id": r#"{"type":"integer"}"#, "from": r#"{"type":"string","enum":["reveal_all","transparency"]}"#; required: ["id", "from"]), read_only: false },
    CommandInfo { name: "mask.invert", summary: "Invert a layer mask.", args: schema!("id": r#"{"type":"integer"}"#; required: ["id"]), read_only: false },
    CommandInfo { name: "mask.enable", summary: "Turn a layer mask on or off.", args: schema!("id": r#"{"type":"integer"}"#, "enabled": r#"{"type":"boolean"}"#; required: ["id", "enabled"]), read_only: false },
    CommandInfo { name: "mask.delete", summary: "Delete a layer mask.", args: schema!("id": r#"{"type":"integer"}"#; required: ["id"]), read_only: false },
    CommandInfo { name: "project.info", summary: "The project's name, file, whether it has unsaved changes, the box type, counts, and the folders AI may read and write.", args: schema!(; required: []), read_only: true },
    CommandInfo { name: "project.new", summary: "Start a new project (in a new tab when the current one has content).", args: schema!(; required: []), read_only: false },
    CommandInfo { name: "project.open", summary: "Open a project file (.pknb). AI can only open files inside the allowed folders.", args: schema!("path": r#"{"type":"string"}"#; required: ["path"]), read_only: false },
    CommandInfo { name: "project.save_as", summary: "Save the project to a new file (.pknb). AI can only write inside the allowed folders and never overwrites an existing file.", args: schema!("path": r#"{"type":"string"}"#; required: ["path"]), read_only: false },
    CommandInfo { name: "finish.add", summary: "Add a finish to an artwork layer (image, text or shape): kind foil (default), emboss, spot_uv (clear varnish only in that shape; matte true for matte varnish) or white (white ink under the colours). It takes the artwork's shape and moves with it; hide the artwork to stamp foil without ink or to blind-emboss. Foil and emboss on the same artwork make raised foil. Foil: foil = gold (default), silver, copper, red_gold, matte_gold, matte_silver, hologram; wave = small waves in the pressed foil, 0 (flat like a mirror) to 100, default 20. Emboss: height (mm, 0.05-2, default 0.4), edge (slope width in mm, 0.1-3, default 0.6), deboss (true presses in). Returns the finish id.", args: schema!("id": r#"{"type":"integer"}"#, "kind": r#"{"type":"string","enum":["foil","emboss","spot_uv","white"]}"#, "matte": r#"{"type":"boolean"}"#, "foil": r#"{"type":"string"}"#, "wave": r#"{"type":"integer"}"#, "height": r#"{"type":"number"}"#, "edge": r#"{"type":"number"}"#, "deboss": r#"{"type":"boolean"}"#; required: ["id"]), read_only: false },
    CommandInfo { name: "finish.set", summary: "Change a finish layer: foil type, waviness (0-100), emboss height, edge and deboss, spot UV matte, visibility or name.", args: schema!("id": r#"{"type":"integer"}"#, "matte": r#"{"type":"boolean"}"#, "foil": r#"{"type":"string"}"#, "wave": r#"{"type":"integer"}"#, "height": r#"{"type":"number"}"#, "edge": r#"{"type":"number"}"#, "deboss": r#"{"type":"boolean"}"#, "visible": r#"{"type":"boolean"}"#, "name": r#"{"type":"string"}"#; required: ["id"]), read_only: false },
    CommandInfo { name: "finish.delete", summary: "Remove a finish layer.", args: schema!("id": r#"{"type":"integer"}"#; required: ["id"]), read_only: false },
    CommandInfo { name: "material.set", summary: "Set the paper (coated_board, coated, matte_coated, uncoated, kraft, corrugated) and the coating on the outside (none, matte_varnish, gloss_varnish, matte_pp, gloss_pp, soft_touch_pp). Either can be left out.", args: schema!("paper": r#"{"type":"string"}"#, "coat": r#"{"type":"string"}"#; required: []), read_only: false },
    CommandInfo { name: "shape.set", summary: "Change the box type: tuck_box, gable, pillow (with a handle), plain_pillow, or prism (with sides 3, 5, 6 or 8). Placed artwork comes off the box (undo brings it back). Returns the new panel names.", args: schema!("kind": r#"{"type":"string","enum":["tuck_box","gable","pillow","plain_pillow","prism"]}"#, "sides": r#"{"type":"integer"}"#; required: ["kind"]), read_only: false },
    CommandInfo { name: "shape.dimensions", summary: "Set inside dimensions in mm (width, depth, height, thickness of the paper; bulge in degrees for pillow; roof for gable; glue_flap, dust_flap, tuck_flap for the tuck-end box, a number or \"auto\"). Values are kept in range. Not undoable.", args: schema!("width": r#"{"type":"number"}"#, "depth": r#"{"type":"number"}"#, "height": r#"{"type":"number"}"#, "thickness": r#"{"type":"number"}"#, "bulge": r#"{"type":"number"}"#, "roof": r#"{"type":"number"}"#, "glue_flap": "{}", "dust_flap": "{}", "tuck_flap": "{}"; required: []), read_only: false },
    CommandInfo { name: "asset.list", summary: "List the assets that can be placed (built-in samples and this project's assets) with their index.", args: schema!(; required: []), read_only: true },
    CommandInfo { name: "asset.import", summary: "Import an image (PNG, JPEG, PSD, SVG) into this project's assets. AI can only read files inside the allowed folders. Returns the asset index.", args: schema!("path": r#"{"type":"string"}"#; required: ["path"]), read_only: false },
    CommandInfo { name: "decal.add", summary: "Place artwork. Give exactly one of: asset (index from asset.list), builtin (logo or band), text ({text, font, size mm, color #rrggbb, align, spacing, leading}), shape ({kind rect|ellipse, fill, stroke, stroke_width, radius}). Optional: panel (name), at (0..1 in the panel), size (mm), angle (degrees), back. Returns the new layer.", args: schema!("asset": r#"{"type":"integer"}"#, "builtin": r#"{"type":"string","enum":["logo","band"]}"#, "text": r#"{"type":"object"}"#, "shape": r#"{"type":"object"}"#, "panel": r#"{"type":"string"}"#, "at": r#"{"type":"array","items":{"type":"number"}}"#, "size": r#"{"type":"array","items":{"type":"number"}}"#, "angle": r#"{"type":"number"}"#, "back": r#"{"type":"boolean"}"#; required: []), read_only: false },
    CommandInfo { name: "text.set", summary: "Change the text or its style (text, font, size mm, color #rrggbb, align left|center|right, spacing, leading). Omitted values stay.", args: schema!("id": r#"{"type":"integer"}"#, "text": r#"{"type":"string"}"#, "font": r#"{"type":"string"}"#, "size": r#"{"type":"number"}"#, "color": r#"{"type":"string"}"#, "align": r#"{"type":"string"}"#, "spacing": r#"{"type":"number"}"#, "leading": r#"{"type":"number"}"#; required: ["id"]), read_only: false },
    CommandInfo { name: "shape_art.set", summary: "Change a shape (kind rect|ellipse, fill and stroke as #rrggbb or null for none, stroke_width mm, radius mm). Omitted values stay.", args: schema!("id": r#"{"type":"integer"}"#, "kind": r#"{"type":"string"}"#, "fill": "{}", "stroke": "{}", "stroke_width": r#"{"type":"number"}"#, "radius": r#"{"type":"number"}"#; required: ["id"]), read_only: false },
    CommandInfo { name: "export.psd", summary: "Save the dieline as a layered PSD (and the back as <name>_back.psd when there is artwork on the back). Finishes become spot color channels, with <name>_finishes.txt beside it. dpi (100, 300, 350 or 600) and bleed (mm, 0-5) change the output settings. AI can only write inside the allowed folders and never overwrites.", args: schema!("path": r#"{"type":"string"}"#, "dpi": r#"{"type":"integer","enum":[100,300,350,600]}"#, "bleed": r#"{"type":"number"}"#; required: ["path"]), read_only: false },
    CommandInfo { name: "export.pdf", summary: "Save the full-size dieline as a PDF tiled on A4 or A3 sheets. AI can only write inside the allowed folders and never overwrites.", args: schema!("path": r#"{"type":"string"}"#; required: ["path"]), read_only: false },
    CommandInfo { name: "export.print_ready", summary: "Save a print-ready PDF for a print shop: one full-size page per side with the artwork, cut lines (spot color Die Cut), fold lines (Crease) and each finish as a spot color plate with overprint, plus a page of finishing instructions. dpi (100, 300, 350 or 600) and bleed (mm, 0-5) change the output settings. AI can only write inside the allowed folders and never overwrites.", args: schema!("path": r#"{"type":"string"}"#, "dpi": r#"{"type":"integer","enum":[100,300,350,600]}"#, "bleed": r#"{"type":"number"}"#; required: ["path"]), read_only: false },
    CommandInfo { name: "render.preview", summary: "Look at the flat dieline with the artwork as a PNG (front or back side, longest edge up to max pixels). Use it to check how the design looks.", args: schema!("side": r#"{"type":"string","enum":["front","back"]}"#, "max": r#"{"type":"integer"}"#; required: []), read_only: true },
    CommandInfo { name: "version.list", summary: "List the kept versions of the project (oldest first): name, time, whether it was kept automatically, and the number of layers.", args: schema!(; required: []), read_only: true },
    CommandInfo { name: "version.keep", summary: "Keep the project as it is now as a named version, so the user can compare or go back to it later.", args: schema!("name": r#"{"type":"string"}"#; required: ["name"]), read_only: false },
    CommandInfo { name: "edit.undo", summary: "Undo the last change.", args: schema!(; required: []), read_only: false },
    CommandInfo { name: "edit.redo", summary: "Redo the change that was undone.", args: schema!(; required: []), read_only: false },
];

/// 続けて呼んだときに、記録を 1 件にまとめるコマンド（値を決めるもの）。
const MERGED: &[&str] = &["layer.opacity.set", "layer.fill.set", "layer.blend.set", "decal.transform", "adjust.set", "layer.rename"];

pub(crate) fn info(name: &str) -> Option<&'static CommandInfo> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// 引数の読み取り。
pub(super) fn int(args: &Value, key: &str) -> Result<i64, String> {
    args.get(key).and_then(Value::as_i64).ok_or_else(|| t!("\"{key}\" (a number) is missing"))
}

fn boolean(args: &Value, key: &str) -> Result<bool, String> {
    args.get(key).and_then(Value::as_bool).ok_or_else(|| t!("\"{key}\" (true or false) is missing"))
}

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| t!("\"{key}\" (text) is missing"))
}

pub(super) fn pair(value: &Value) -> Option<[f32; 2]> {
    let list = value.as_array()?;
    match list.as_slice() {
        [a, b] => Some([a.as_f64()? as f32, b.as_f64()? as f32]),
        _ => None,
    }
}

/// 種類の名前（コマンドの結果に使う。画面の言葉ではない）。
fn kind_of(picture: &Picture) -> &'static str {
    match picture {
        Picture::Logo => "logo",
        Picture::Band => "band",
        Picture::Image { vector: true, .. } => "vector_image",
        Picture::Image { .. } => "image",
        Picture::Text(_) => "text",
        Picture::Shape(_) => "shape",
        Picture::Group { .. } => "group",
        Picture::Adjustment => "adjustment_layer",
    }
}

impl App {
    /// レイヤーに印を付ける（まだないもの、重なったもの）。開いたとき、足したときに呼ぶ。
    pub(super) fn ensure_ids(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut next = self.decals.iter().map(|d| d.layer.id).max().unwrap_or(0);
        for placed in &mut self.decals {
            if placed.layer.id == 0 || !seen.insert(placed.layer.id) {
                next += 1;
                placed.layer.id = next;
                seen.insert(next);
            }
        }
    }

    pub(super) fn index_of(&self, id: i64) -> Result<usize, String> {
        self.decals.iter().position(|d| i64::from(d.layer.id) == id).ok_or_else(|| t!("There is no layer with id {id}"))
    }

    fn layer_arg(&self, args: &Value) -> Result<usize, String> {
        self.index_of(int(args, "id")?)
    }

    fn layers_arg(&self, args: &Value) -> Result<Vec<usize>, String> {
        let list = args.get("ids").and_then(Value::as_array).ok_or_else(|| t!("\"ids\" (a list of layer ids) is missing").to_owned())?;
        list.iter().map(|v| v.as_i64().ok_or_else(|| t!("Layer ids are numbers").to_owned()).and_then(|id| self.index_of(id))).collect()
    }

    fn unlocked(&self, index: usize) -> Result<(), String> {
        if self.decals[index].layer.locked { Err(t!("Layer {} is locked", self.decals[index].layer.id)) } else { Ok(()) }
    }

    /// レイヤー 1 つを JSON に。
    pub(super) fn layer_json(&self, index: usize) -> Value {
        let placed = &self.decals[index];
        let layer = &placed.layer;
        let mut out = json!({
            "id": layer.id,
            "kind": kind_of(&placed.picture),
            "label": self.layer_label(index),
            "side": if placed.back { "back" } else { "front" },
            "depth": layer.depth,
            "visible": !layer.hidden,
            "locked": layer.locked,
            "lock_pixels": layer.lock_pixels,
            "lock_position": layer.lock_position,
            "opacity": layer.opacity,
            "blend": layer.blend.key(),
            "clipped": layer.clipped,
        });
        if let Some(name) = &layer.name {
            out["name"] = json!(name);
        }
        if !matches!(placed.picture, Picture::Group { .. }) {
            out["fill"] = json!(layer.fill);
        }
        if let Picture::Group { open } = placed.picture {
            out["open"] = json!(open);
        }
        if !layer.adjust.is_empty() {
            out["adjustments"] = Value::Array(layer.adjust.iter().map(adjustment_to_json).collect());
        }
        match &placed.picture {
            Picture::Text(art) => out["text"] = super::commands_doc::text_json(&art.spec),
            Picture::Shape(spec) => out["shape"] = super::commands_doc::shape_json(spec),
            _ => {}
        }
        if let Some(mask) = &layer.mask {
            out["mask"] = json!({ "enabled": mask.enabled, "inverted": mask.invert });
        }
        if crate::paint::is_artwork(&placed.picture) {
            let d = &placed.decal;
            let panel = self.template.panels.get(d.panel).map_or("", |p| p.name);
            out["panel"] = json!(panel);
            out["at"] = json!([round(d.at[0], 4), round(d.at[1], 4)]);
            out["size"] = json!([round(d.size[0], 2), round(d.size[1], 2)]);
            out["angle"] = json!(round(d.angle.to_degrees(), 2));
        }
        out
    }

    /// 案件の状態を JSON に（`doc.inspect`）。
    pub(super) fn inspect(&self) -> Value {
        let d = &self.dims;
        json!({
            "file": self.file.path.as_ref().map(|p| p.display().to_string()),
            "shape": { "summary": self.shape_summary(), "sides": d.sides, "ecma": pakenoba_core::ecma::Code::from_u32(d.ecma).map(|c| c.name()), "w": d.w, "d": d.d, "h": d.h, "paper_mm": d.t },
            "material": { "paper": self.material.paper.key(), "coat": self.material.coat.key() },
            "finishes": self.finishes_json(),
            "panels": self.template.panels.iter().map(|p| p.name).collect::<Vec<_>>(),
            "layers": (0..self.decals.len()).map(|i| self.layer_json(i)).collect::<Vec<_>>(),
            "selected": self.selection().iter().map(|&i| self.decals[i].layer.id).collect::<Vec<_>>(),
            "note": "Layers are listed from bottom to top. A group's contents come just before its header and are deeper by one.",
        })
    }

    /// コマンドを 1 つ実行する。`actor` は誰が呼んだか（`person`、`agent:<名前>`、`mcp:<名前>`）。
    /// 変えるコマンドは、元に戻す 1 回ぶんになり、操作の記録に残る。
    pub(crate) fn run_command(&mut self, actor: &str, name: &str, args: Value) -> Result<Value, String> {
        let info = info(name).ok_or_else(|| t!("Unknown command \"{name}\""))?;
        self.ensure_ids();
        self.actor = actor.to_owned();
        let result = self.dispatch(name, &args);
        self.actor = "person".to_owned();
        let result = result?;
        self.ensure_ids();
        // 案件を開く、作る、保存するは、案件の中身を変えないので記録しない。
        if !info.read_only && !name.starts_with("project.") {
            (self.art_dirty, self.back_dirty) = (true, true);
            self.log_op(actor, name, args);
        }
        Ok(result)
    }

    /// いくつかのコマンドを続けて実行する。元に戻すのは、まとめて 1 回ぶん。途中で失敗したら、そこで止める（そこまでの変更は残り、元に戻せる）。
    pub(crate) fn run_batch(&mut self, actor: &str, calls: Vec<(String, Value)>) -> Result<Vec<Value>, String> {
        // すでに外側でまとめている（エージェントの 1 回の依頼）なら、その中に入れる。
        let outer = self.undo_batch.is_some();
        if !outer {
            self.undo_batch = Some(false);
        }
        let mut results = Vec::new();
        let mut failed = None;
        for (name, args) in calls {
            match self.run_command(actor, &name, args) {
                Ok(value) => results.push(value),
                Err(e) => {
                    failed = Some(t!("{name}: {e} ({} command(s) before it were done)", results.len()));
                    break;
                }
            }
        }
        if !outer {
            self.undo_batch = None;
        }
        match failed {
            Some(e) => Err(e),
            None => Ok(results),
        }
    }

    /// 操作の記録に 1 件足す。続けて同じ人が同じものを同じコマンドで変えたときは、前の 1 件を書き換える。
    pub(crate) fn log_op(&mut self, actor: &str, command: &str, args: Value) {
        if !self.settings.record_log {
            return;
        }
        let time = super::file::now_local();
        if let Some(last) = self.log.last_mut()
            && MERGED.contains(&command)
            && last.actor == actor
            && last.command == command
            && last.args.get("ids").or(last.args.get("id")) == args.get("ids").or(args.get("id"))
            && last.args.get("index") == args.get("index")
        {
            (last.time, last.args) = (time, args);
            return;
        }
        self.log.push(LogEntry { time, actor: actor.to_owned(), command: command.to_owned(), args });
        // 増えすぎたら、古いものを捨てる（保存するときも、新しい LOG_LIMIT 件だけ書く）。
        if self.log.len() > project::LOG_LIMIT * 2 {
            self.log.drain(..self.log.len() - project::LOG_LIMIT);
        }
    }

    fn dispatch(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "doc.inspect" => Ok(self.inspect()),
            "layer.list" => Ok(Value::Array((0..self.decals.len()).map(|i| self.layer_json(i)).collect())),
            "layer.select" => {
                let list = self.layers_arg(args)?;
                self.select_only(list.first().copied());
                self.extra = list.iter().skip(1).copied().collect();
                Ok(Value::Null)
            }
            "layer.rename" => {
                let index = self.layer_arg(args)?;
                let name = text(args, "name")?.trim().to_owned();
                self.checkpoint();
                self.decals[index].layer.name = (!name.is_empty()).then_some(name);
                Ok(Value::Null)
            }
            "layer.visible" => {
                let list = self.layers_arg(args)?;
                let visible = boolean(args, "visible")?;
                self.checkpoint();
                for i in list {
                    self.decals[i].layer.hidden = !visible;
                }
                Ok(Value::Null)
            }
            "layer.lock" => {
                let list = self.layers_arg(args)?;
                let [all, pixels, position] = ["all", "pixels", "position"].map(|k| args.get(k).and_then(Value::as_bool));
                if all.is_none() && pixels.is_none() && position.is_none() {
                    return Err(t!("Give at least one of \"all\", \"pixels\" and \"position\"").to_owned());
                }
                self.checkpoint();
                for i in list {
                    let l = &mut self.decals[i].layer;
                    l.locked = all.unwrap_or(l.locked);
                    l.lock_pixels = pixels.unwrap_or(l.lock_pixels);
                    l.lock_position = position.unwrap_or(l.lock_position);
                }
                self.grab = None;
                if self.brush_target().is_none() {
                    self.brush = None;
                }
                Ok(Value::Null)
            }
            "layer.blend.set" => {
                let list = self.layers_arg(args)?;
                let key = text(args, "blend")?;
                let blend = Blend::from_key(key).ok_or_else(|| t!("Unknown blend mode \"{key}\""))?;
                for &i in &list {
                    self.unlocked(i)?;
                    if blend == Blend::PassThrough && !matches!(self.decals[i].picture, Picture::Group { .. }) {
                        return Err(t!("Only groups can use pass through").to_owned());
                    }
                }
                self.checkpoint();
                for i in list {
                    self.decals[i].layer.blend = blend;
                }
                Ok(Value::Null)
            }
            "layer.opacity.set" | "layer.fill.set" => {
                let list = self.layers_arg(args)?;
                let key = if name == "layer.opacity.set" { "opacity" } else { "fill" };
                let value = int(args, key)?.clamp(0, 100) as u8;
                for &i in &list {
                    self.unlocked(i)?;
                }
                self.checkpoint();
                for i in list {
                    let is_group = matches!(self.decals[i].picture, Picture::Group { .. });
                    let l = &mut self.decals[i].layer;
                    if key == "opacity" {
                        l.opacity = value;
                    } else if !is_group {
                        l.fill = value;
                    }
                }
                Ok(Value::Null)
            }
            "layer.reorder" => {
                let list = self.layers_arg(args)?;
                let step = int(args, "step")?;
                if step != 1 && step != -1 {
                    return Err(t!("\"step\" is 1 (up) or -1 (down)").to_owned());
                }
                let ids: Vec<u32> = list.iter().map(|&i| self.decals[i].layer.id).collect();
                self.select_only(list.first().copied());
                self.extra = list.iter().skip(1).copied().collect();
                self.reorder_selected(step as isize);
                Ok(json!({ "order": ids }))
            }
            "layer.clip" => {
                let index = self.layer_arg(args)?;
                let wanted = boolean(args, "clipped")?;
                if self.decals[index].layer.clipped != wanted {
                    self.toggle_clip(index);
                }
                let index = self.layer_arg(args)?;
                if self.decals[index].layer.clipped != wanted {
                    return Err(t!("There is no layer just below it in the same group to clip to.").to_owned());
                }
                Ok(Value::Null)
            }
            "group.create" => {
                let list = self.layers_arg(args)?;
                self.select_only(list.first().copied());
                self.extra = list.iter().skip(1).copied().collect();
                let before = self.decals.len();
                self.group_selected();
                if self.decals.len() == before {
                    return Err(t!("Could not make the group (groups can be nested up to {} levels)", project::MAX_DEPTH));
                }
                self.ensure_ids();
                let header = self.selected.ok_or_else(|| t!("Could not make the group").to_owned())?;
                Ok(json!({ "id": self.decals[header].layer.id }))
            }
            "group.ungroup" => {
                let index = self.layer_arg(args)?;
                if !matches!(self.decals[index].picture, Picture::Group { .. }) {
                    return Err(t!("Layer {} is not a group", self.decals[index].layer.id));
                }
                self.ungroup(index);
                Ok(Value::Null)
            }
            "decal.delete" => {
                let list = self.layers_arg(args)?;
                self.select_only(list.first().copied());
                self.extra = list.iter().skip(1).copied().collect();
                self.delete_selected();
                Ok(Value::Null)
            }
            "decal.transform" => {
                let index = self.layer_arg(args)?;
                if !crate::paint::is_artwork(&self.decals[index].picture) {
                    return Err(t!("Groups and adjustment layers have no position").to_owned());
                }
                let layer = &self.decals[index].layer;
                if layer.locked || layer.lock_position {
                    return Err(t!("The position of layer {} is locked", layer.id));
                }
                let mut decal = self.decals[index].decal;
                if let Some(panel) = args.get("panel") {
                    let panel = panel.as_str().ok_or_else(|| t!("\"panel\" is a panel name").to_owned())?;
                    decal.panel = self.template.panels.iter().position(|p| p.name == panel).ok_or_else(|| t!("There is no panel named \"{panel}\""))?;
                }
                if let Some(at) = args.get("at") {
                    decal.at = pair(at).ok_or_else(|| t!("\"at\" is two numbers").to_owned())?;
                }
                if let Some(size) = args.get("size") {
                    let size = pair(size).filter(|s| s[0] > 0.0 && s[1] > 0.0).ok_or_else(|| t!("\"size\" is two numbers above 0 (mm)").to_owned())?;
                    decal.size = size;
                }
                if let Some(angle) = args.get("angle") {
                    decal.angle = (angle.as_f64().ok_or_else(|| t!("\"angle\" is a number (degrees)").to_owned())? as f32).to_radians();
                }
                let back = args.get("back").and_then(Value::as_bool);
                self.checkpoint();
                self.decals[index].decal = decal;
                if let Some(back) = back {
                    self.decals[index].back = back;
                }
                Ok(self.layer_json(index))
            }
            "adjust.add" => {
                let index = self.layer_arg(args)?;
                if !crate::paint::is_artwork(&self.decals[index].picture) {
                    return Err(t!("Adjustments for one artwork can only be added to artwork. Use adjustment_layer.add instead.").to_owned());
                }
                self.sealed(index)?;
                let adjust = adjustment_arg(args)?;
                self.checkpoint();
                self.decals[index].layer.adjust.push(adjust);
                Ok(json!({ "index": self.decals[index].layer.adjust.len() - 1 }))
            }
            "adjust.set" | "adjust.toggle" | "adjust.remove" | "adjust.move" => {
                let index = self.layer_arg(args)?;
                self.sealed(index)?;
                let at = int(args, "index")?;
                let count = self.decals[index].layer.adjust.len();
                let at = usize::try_from(at).ok().filter(|&a| a < count).ok_or_else(|| t!("Layer {} has no adjustment number {at}", self.decals[index].layer.id))?;
                let new = if name == "adjust.set" { Some(adjustment_arg(args)?) } else { None };
                let on = if name == "adjust.toggle" { Some(boolean(args, "on")?) } else { None };
                let to = if name == "adjust.move" { Some(usize::try_from(int(args, "to")?).ok().filter(|&t| t < count).ok_or_else(|| t!("\"to\" is out of range").to_owned())?) } else { None };
                self.checkpoint();
                let list = &mut self.decals[index].layer.adjust;
                match name {
                    "adjust.set" => {
                        let on = list[at].on;
                        list[at] = Adjustment { on, ..new.unwrap() };
                    }
                    "adjust.toggle" => list[at].on = on.unwrap(),
                    "adjust.move" => {
                        let item = list.remove(at);
                        list.insert(to.unwrap(), item);
                    }
                    _ => {
                        list.remove(at);
                        (self.adjust_focus, self.effect_focus) = (None, None);
                    }
                }
                Ok(Value::Null)
            }
            "adjustment_layer.add" => {
                let adjust = adjustment_arg(args)?;
                if !adjust.kind.per_pixel() {
                    return Err(t!("This adjustment works on one picture as a whole, so it cannot be an adjustment layer").to_owned());
                }
                match args.get("above") {
                    Some(above) => {
                        let id = above.as_i64().ok_or_else(|| t!("\"above\" is a layer id").to_owned())?;
                        let index = self.index_of(id)?;
                        self.select_only(Some(index));
                    }
                    None => self.select_only(None),
                }
                self.add_adjustment_layer(adjust.kind);
                self.ensure_ids();
                let at = self.selected.ok_or_else(|| t!("Could not add the adjustment layer").to_owned())?;
                Ok(json!({ "id": self.decals[at].layer.id }))
            }
            "mask.add" => {
                let index = self.layer_arg(args)?;
                if !crate::paint::is_artwork(&self.decals[index].picture) {
                    return Err(t!("Masks can only be added to artwork").to_owned());
                }
                self.sealed(index)?;
                let how = match text(args, "from")? {
                    "reveal_all" => MaskFrom::RevealAll,
                    "transparency" => MaskFrom::Transparency,
                    other => return Err(t!("Unknown way to make a mask \"{other}\"")),
                };
                self.select_only(Some(index));
                self.add_mask(how);
                if self.decals[index].layer.mask.is_none() {
                    return Err(t!("Could not make a mask for this artwork").to_owned());
                }
                Ok(Value::Null)
            }
            "mask.invert" | "mask.enable" | "mask.delete" => {
                let index = self.layer_arg(args)?;
                self.sealed(index)?;
                if self.decals[index].layer.mask.is_none() {
                    return Err(t!("Layer {} has no mask", self.decals[index].layer.id));
                }
                let enabled = if name == "mask.enable" { Some(boolean(args, "enabled")?) } else { None };
                self.checkpoint();
                let layer = &mut self.decals[index].layer;
                match name {
                    "mask.invert" => {
                        if let Some(m) = &mut layer.mask {
                            m.invert = !m.invert;
                        }
                    }
                    "mask.enable" => {
                        if let Some(m) = &mut layer.mask {
                            m.enabled = enabled.unwrap();
                        }
                    }
                    _ => {
                        layer.mask = None;
                        self.brush = None;
                    }
                }
                Ok(Value::Null)
            }
            "edit.undo" => {
                if !self.history.can_undo() {
                    return Err(t!("Nothing to undo").to_owned());
                }
                self.undo();
                Ok(Value::Null)
            }
            "edit.redo" => {
                if !self.history.can_redo() {
                    return Err(t!("Nothing to redo").to_owned());
                }
                self.redo();
                Ok(Value::Null)
            }
            _ => self.dispatch_doc(name, args),
        }
    }

    fn sealed(&self, index: usize) -> Result<(), String> {
        if self.decals[index].layer.sealed() { Err(t!("The image of layer {} is locked", self.decals[index].layer.id)) } else { Ok(()) }
    }
}

/// 画面の操作から呼ぶもの。人（`person`）の操作として、コマンドを通す（記録に残る）。
impl App {
    /// 人の操作としてコマンドを呼ぶ。できなかったら、下の帯に理由を出す。
    pub(super) fn person(&mut self, name: &str, args: Value) -> Option<Value> {
        match self.run_command("person", name, args) {
            Ok(value) => Some(value),
            Err(e) => {
                self.say(e);
                None
            }
        }
    }

    fn selected_ids(&mut self) -> Vec<u32> {
        self.ensure_ids();
        self.selection().iter().map(|&i| self.decals[i].layer.id).collect()
    }

    pub(super) fn cmd_delete(&mut self) {
        let ids = self.selected_ids();
        if !ids.is_empty() {
            self.person("decal.delete", json!({ "ids": ids }));
        }
    }

    pub(super) fn cmd_group(&mut self) {
        let ids = self.selected_ids();
        self.person("group.create", json!({ "ids": ids }));
    }

    pub(super) fn cmd_ungroup(&mut self, index: usize) {
        self.ensure_ids();
        let id = self.decals[index].layer.id;
        self.person("group.ungroup", json!({ "id": id }));
    }

    pub(super) fn cmd_clip(&mut self, index: usize) {
        if index >= self.decals.len() || matches!(self.decals[index].picture, Picture::Group { .. }) {
            return;
        }
        self.ensure_ids();
        let layer = &self.decals[index].layer;
        let (id, clipped) = (layer.id, !layer.clipped);
        self.person("layer.clip", json!({ "id": id, "clipped": clipped }));
    }

    pub(super) fn cmd_reorder(&mut self, step: isize) {
        let ids = self.selected_ids();
        if !ids.is_empty() {
            self.person("layer.reorder", json!({ "ids": ids, "step": step }));
        }
    }

    /// 柄だけに効く調整を足す（いくつ選んでいても）。
    pub(super) fn cmd_adjust(&mut self, kind: project::AdjustKind) {
        let ids = self.selected_ids();
        let before: usize = self.decals.iter().map(|d| d.layer.adjust.len()).sum();
        self.add_adjustment(kind);
        if self.decals.iter().map(|d| d.layer.adjust.len()).sum::<usize>() > before {
            self.log_op("person", "adjust.add", json!({ "ids": ids, "adjustment": adjustment_to_json(&Adjustment::new(kind)) }));
        }
    }

    pub(super) fn cmd_adjust_layer(&mut self, kind: project::AdjustKind) {
        self.add_adjustment_layer(kind);
        self.ensure_ids();
        if let Some(at) = self.selected {
            let id = self.decals[at].layer.id;
            self.log_op("person", "adjustment_layer.add", json!({ "id": id, "adjustment": adjustment_to_json(&Adjustment::new(kind)) }));
        }
    }

    pub(super) fn cmd_mask(&mut self, how: MaskFrom) {
        let ids = self.selected_ids();
        self.add_mask(how);
        let from = match how {
            MaskFrom::RevealAll => "reveal_all",
            MaskFrom::Transparency => "transparency",
            MaskFrom::Subject => "subject",
        };
        if let Some(&id) = ids.first() {
            self.log_op("person", "mask.add", json!({ "id": id, "from": from }));
        }
    }

    /// 柄の場所、大きさ、向きを変え終えたときの記録（ドラッグを離したとき、数値の欄を変えたとき）。
    pub(super) fn log_transform(&mut self) {
        self.ensure_ids();
        for index in self.selection() {
            let mut args = self.layer_json(index);
            let keep = ["id", "panel", "at", "size", "angle", "side"];
            if let Value::Object(map) = &mut args {
                map.retain(|k, _| keep.contains(&k.as_str()));
            }
            self.log_op("person", "decal.transform", args);
        }
    }

    /// 柄を足したときの記録。
    pub(super) fn log_added(&mut self, index: usize) {
        self.ensure_ids();
        let mut args = self.layer_json(index);
        if let Value::Object(map) = &mut args {
            map.retain(|k, _| ["id", "kind", "label", "panel", "side"].contains(&k.as_str()));
        }
        self.log_op("person", "decal.add", args);
    }
}

fn adjustment_arg(args: &Value) -> Result<Adjustment, String> {
    adjustment_from_json(args.get("adjustment").ok_or_else(|| t!("\"adjustment\" is missing").to_owned())?)
}

fn round(v: f32, digits: i32) -> f64 {
    let k = 10f64.powi(digits);
    (f64::from(v) * k).round() / k
}

#[cfg(test)]
mod tests {
    use super::super::{Initial, Placed};
    use super::*;
    use pakenoba_core::project::{ShapeKind, ShapeSpec};
    use pakenoba_core::surface::Decal;

    fn app() -> App {
        let mut app = App::new(&Initial::default());
        app.decals.clear();
        let panel = app.template.panels.iter().position(|p| p.name == "正面").unwrap();
        for c in [[200, 0, 0], [0, 0, 200]] {
            let spec = ShapeSpec { kind: ShapeKind::Rect, fill: Some(c), stroke: None, stroke_width: 1.0, radius: 0.0 };
            app.decals.push(Placed { decal: Decal { panel, at: [0.5, 0.5], size: [30.0, 30.0], angle: 0.0 }, picture: Picture::Shape(spec), back: false, layer: Default::default() });
        }
        app.ensure_ids();
        app
    }

    #[test]
    fn コマンドの一覧は_名前が重ならず_引数の形はjson() {
        let mut names = std::collections::HashSet::new();
        for c in COMMANDS {
            assert!(names.insert(c.name), "{}", c.name);
            assert!(!c.summary.is_empty());
            if !c.read_only {
                assert_ne!(super::super::oplog::command_label(c.name), c.name, "画面に出す名前がない: {}", c.name);
            }
            let schema: Value = serde_json::from_str(c.args).unwrap_or_else(|e| panic!("{}: {e}", c.name));
            assert_eq!(schema["type"], "object");
        }
    }

    #[test]
    fn idで指して変え_元に戻せ_記録に残る() {
        let mut app = app();
        let layers = app.run_command("person", "layer.list", Value::Null).unwrap();
        let ids: Vec<i64> = layers.as_array().unwrap().iter().map(|l| l["id"].as_i64().unwrap()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|&id| id > 0) && ids[0] != ids[1]);
        app.run_command("agent:test", "layer.blend.set", json!({ "ids": [ids[1]], "blend": "multiply" })).unwrap();
        let index = app.index_of(ids[1]).unwrap();
        assert_eq!(app.decals[index].layer.blend, Blend::Multiply);
        assert_eq!(app.log.len(), 1);
        assert_eq!((app.log[0].actor.as_str(), app.log[0].command.as_str()), ("agent:test", "layer.blend.set"));
        // 読むだけのコマンドは、記録に残さない。
        app.run_command("agent:test", "doc.inspect", Value::Null).unwrap();
        assert_eq!(app.log.len(), 1);
        // 元に戻すと、元の描画モード。
        app.run_command("agent:test", "edit.undo", Value::Null).unwrap();
        assert_eq!(app.decals[index].layer.blend, Blend::Normal);
        // 知らないものは、変えずに理由を返す。
        assert!(app.run_command("agent:test", "layer.blend.set", json!({ "ids": [ids[1]], "blend": "sparkle" })).is_err());
        assert!(app.run_command("agent:test", "layer.visible", json!({ "ids": [999], "visible": false })).is_err());
        assert!(app.run_command("agent:test", "no.such", Value::Null).is_err());
    }

    #[test]
    fn ベクターで隠した色には_白押しをしない() {
        use pakenoba_core::project::{ColorStyle, VectorStyle};
        let mut app = app();
        // 左半分が赤、右半分が青の SVG（40 × 20 mm）。
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="20mm" viewBox="0 0 40 20"><rect x="0" y="0" width="20" height="20" fill="#ff0000"/><rect x="20" y="0" width="20" height="20" fill="#0000ff"/></svg>"##;
        let (w, h, rgba) = crate::vector::decode_svg(svg).unwrap();
        let rgba = std::sync::Arc::new(rgba);
        crate::vector::remember_svg(&rgba, svg);
        let panel = app.template.panels.iter().position(|p| p.name == "正面").unwrap();
        let decal = pakenoba_core::surface::Decal { panel, at: [0.5, 0.5], size: [40.0, 20.0], angle: 0.0 };
        let layer = pakenoba_core::project::Layer { vector: Some(VectorStyle { colors: vec![ColorStyle { hidden: true, ..ColorStyle::new([0, 0, 255]) }], ..Default::default() }), ..Default::default() };
        app.decals.push(crate::paint::Placed { decal, picture: crate::paint::Picture::Image { w, h, rgba, vector: true }, back: false, layer });
        let index = app.decals.len() - 1;
        app.select_only(Some(index));
        app.add_finish(pakenoba_core::material::FinishKind::White, false);
        // 隠した色を除いた絵は別のスレッドで作るので、できるまで待つ。
        let center = app.decals[index].center(&app.template);
        let at = |app: &App, dx: f32| {
            let maps = app.finish_maps(false).unwrap();
            let f = crate::art::Frame::screen(&app.template);
            let [x, y] = f.to_px([center[0] + dx, center[1]]);
            maps[1][(y as usize * app.sheet.w + x as usize) * 4 + 2]
        };
        for _ in 0..100 {
            if !crate::effects::busy() && at(&app, 10.0) == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(at(&app, -10.0) > 200, "見えている赤の所には白押し");
        assert_eq!(at(&app, 10.0), 0, "隠した青の所には白押しをしない");
    }

    #[test]
    fn 白押しを選んだまま_同じ柄にエンボスを足せる() {
        let mut app = app();
        app.select_only(Some(0));
        app.add_finish(pakenoba_core::material::FinishKind::White, false);
        assert_eq!(app.finishes.len(), 1);
        // 足した白押しを選ぶ（柄の選択は外れる）。
        app.select_only(None);
        app.finish_selected = Some(0);
        assert_eq!(app.finish_target(), Some(0));
        app.add_emboss(false);
        assert_eq!(app.finishes.len(), 2);
        assert_eq!(app.finishes[0].link, app.finishes[1].link);
        // 白版とエンボスの高さが、両方入る。
        let maps = app.finish_maps(app.decals[0].back).unwrap();
        assert!(maps[1].chunks_exact(4).any(|p| p[2] > 200), "白版");
        assert!(maps[0].chunks_exact(4).any(|p| p[3] > 140), "エンボス");
    }

    #[test]
    fn 柄に箔を押し_種類を変え_外せて_元に戻せる() {
        let mut app = app();
        let id = i64::from(app.decals[0].layer.id);
        let added = app.run_command("agent:test", "finish.add", json!({ "id": id, "foil": "silver" })).unwrap();
        let finish = added["id"].as_i64().unwrap();
        assert!(app.decals.iter().all(|d| i64::from(d.layer.id) != finish), "加工の印は、柄の印と重ならない");
        assert_eq!(app.finishes.len(), 1);
        assert_eq!(app.finishes[0].foil, pakenoba_core::material::Foil::Silver);
        // 加工の版の絵には、柄の形のところに箔が入る（柄を隠しても）。
        app.run_command("agent:test", "layer.visible", json!({ "ids": [id], "visible": false })).unwrap();
        let back = app.decals[0].back;
        let map = app.finish_maps(back).expect("加工の版")[0].clone();
        assert!(map.chunks_exact(4).any(|p| p[0] > 200 && p[1] == super::super::finishes::foil_code(pakenoba_core::material::Foil::Silver)));
        app.run_command("agent:test", "finish.set", json!({ "id": finish, "foil": "hologram" })).unwrap();
        assert_eq!(app.finishes[0].foil, pakenoba_core::material::Foil::Hologram);
        let inspect = app.run_command("agent:test", "doc.inspect", Value::Null).unwrap();
        assert_eq!(inspect["finishes"][0]["foil"], "hologram");
        assert!(app.run_command("agent:test", "finish.set", json!({ "id": finish, "foil": "glitter" })).is_err());
        app.run_command("agent:test", "finish.delete", json!({ "id": finish })).unwrap();
        assert!(app.finishes.is_empty());
        app.run_command("agent:test", "edit.undo", Value::Null).unwrap();
        assert_eq!(app.finishes.len(), 1);
        // 同じ柄にエンボスも付けると、版の A に高さが入る（浮き出し箔）。デボスはへこむ。
        let emboss = app.run_command("agent:test", "finish.add", json!({ "id": id, "kind": "emboss", "height": 0.8 })).unwrap()["id"].as_i64().unwrap();
        let map = app.finish_maps(back).unwrap()[0].clone();
        assert!(map.chunks_exact(4).any(|p| p[3] > 160));
        app.run_command("agent:test", "finish.set", json!({ "id": emboss, "deboss": true })).unwrap();
        let map = app.finish_maps(back).unwrap()[0].clone();
        assert!(map.chunks_exact(4).any(|p| p[3] < 96) && map.chunks_exact(4).all(|p| p[3] <= 128));
        assert_eq!(app.run_command("agent:test", "doc.inspect", Value::Null).unwrap()["finishes"][1]["kind"], "emboss");
        app.run_command("agent:test", "finish.delete", json!({ "id": emboss })).unwrap();
        // 案件に保存して開き直しても残る。
        let project = app.project().unwrap();
        assert_eq!(project.finishes.len(), 1);
        assert_eq!(project.finishes[0].link, app.decals[0].layer.id);
    }

    #[test]
    fn まとめて呼ぶと_元に戻すのは1回ぶん() {
        let mut app = app();
        let id = i64::from(app.decals[0].layer.id);
        let calls = vec![
            ("layer.opacity.set".to_owned(), json!({ "ids": [id], "opacity": 40 })),
            ("layer.visible".to_owned(), json!({ "ids": [id], "visible": false })),
            ("decal.transform".to_owned(), json!({ "id": id, "size": [30.0, 20.0], "angle": 90.0 })),
            ("adjust.add".to_owned(), json!({ "id": id, "adjustment": { "kind": "invert" } })),
        ];
        app.run_batch("agent:test", calls).unwrap();
        let layer = &app.decals[0];
        assert_eq!((layer.layer.opacity, layer.layer.hidden, layer.decal.size, layer.layer.adjust.len()), (40, true, [30.0, 20.0], 1));
        assert!((layer.decal.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        app.undo();
        let layer = &app.decals[0];
        assert_eq!((layer.layer.opacity, layer.layer.hidden, layer.layer.adjust.len()), (100, false, 0));
        assert!(!app.history.can_undo(), "まとめて 1 回ぶん");
    }

    #[test]
    fn 続けて同じものを変えた記録は_1件にまとめ_記録しない設定もある() {
        let mut app = app();
        let id = i64::from(app.decals[0].layer.id);
        for opacity in [90, 80, 70] {
            app.run_command("person", "layer.opacity.set", json!({ "ids": [id], "opacity": opacity })).unwrap();
        }
        assert_eq!(app.log.len(), 1);
        assert_eq!(app.log[0].args["opacity"], 70);
        app.settings.record_log = false;
        app.run_command("person", "layer.visible", json!({ "ids": [id], "visible": false })).unwrap();
        assert_eq!(app.log.len(), 1);
    }

    #[test]
    fn グループと調整レイヤーとマスクもコマンドで作れる() {
        let mut app = app();
        let ids: Vec<i64> = app.decals.iter().map(|d| i64::from(d.layer.id)).collect();
        let group = app.run_command("agent:test", "group.create", json!({ "ids": ids })).unwrap()["id"].as_i64().unwrap();
        assert!(matches!(app.decals[app.index_of(group).unwrap()].picture, Picture::Group { .. }));
        let adjust = app.run_command("agent:test", "adjustment_layer.add", json!({ "adjustment": { "kind": "hue_saturation", "hue": 30 }, "above": group })).unwrap()["id"].as_i64().unwrap();
        assert!(matches!(app.decals[app.index_of(adjust).unwrap()].picture, Picture::Adjustment));
        app.run_command("agent:test", "mask.add", json!({ "id": ids[0], "from": "reveal_all" })).unwrap();
        app.run_command("agent:test", "mask.invert", json!({ "id": ids[0] })).unwrap();
        assert!(app.decals[app.index_of(ids[0]).unwrap()].layer.mask.as_ref().unwrap().invert);
        app.run_command("agent:test", "group.ungroup", json!({ "id": group })).unwrap();
        assert!(!app.decals.iter().any(|d| matches!(d.picture, Picture::Group { .. })));
        // 状態を JSON で見られる。
        let doc = app.run_command("agent:test", "doc.inspect", Value::Null).unwrap();
        assert_eq!(doc["layers"].as_array().unwrap().len(), 3);
        assert!(doc["panels"].as_array().unwrap().iter().any(|p| p == "正面"));
    }
}
