//! 3D の書き出し: GLB（glTF 2.0 の 1 ファイル）と OBJ（.obj と .mtl と絵）。
//!
//! いまの開き具合と組み立ての具合の箱の形を、そのまま書き出す。紙の表・裏・断面を別の材質にし、
//! 表と裏には、柄と加工の見た目を焼き込んだ絵を貼る（箔やエンボスの光り方そのものは入らない）。
//! 長さはメートル（glTF の決まり）、上が +Y。

use pakenoba_i18n::t;
use std::path::{Path, PathBuf};

use pakenoba_core::template::Vertex;

use super::App;
use crate::art::alpha_over;
use crate::paint::{Canvas, Marks, paint};

/// 紙の 3 つの面（表、裏、断面）。
const PARTS: [&str; 3] = ["outside", "inside", "edge"];

/// 書き出す形と絵。
pub(super) struct Model {
    /// 面ごとの三角形（3 つずつ）。並びは `PARTS` と同じ。
    parts: [Vec<Vertex>; 3],
    /// 表と裏の絵（RGBA、展開図の向き）と、その大きさ。
    textures: [Vec<u8>; 2],
    size: [usize; 2],
    /// 紙の色（断面）と、表・裏・断面のつやの無さ。
    edge_color: [u8; 3],
    roughness: [f32; 3],
}

impl App {
    /// いまの箱の形と、柄を焼き込んだ絵。
    pub(super) fn model(&self) -> Model {
        let mesh = self.template.build_mesh_at(self.fold, [1.0 - self.open[0], 1.0 - self.open[1]]);
        let mut parts: [Vec<Vertex>; 3] = Default::default();
        for tri in mesh.chunks_exact(3) {
            let part = (tri[0].kind.round() as usize).min(2);
            parts[part].extend_from_slice(tri);
        }
        let colors = crate::gpu::paper_colors(self.material.paper);
        let (w, h) = (self.sheet.w, self.sheet.h);
        let texture = |back: bool| {
            let paper = colors[usize::from(back)].0;
            let mut rgba = [paper[0], paper[1], paper[2], 255].repeat(w * h);
            if let Some(custom) = self.custom[usize::from(back)].as_deref() {
                alpha_over(&mut rgba, custom);
            }
            paint(&self.surface, &self.template, &self.decals, Marks::default(), back, Canvas::output(w, h, &mut rgba));
            if let Some(maps) = self.finish_maps(back) {
                self.overlay_finishes(&mut rgba, &maps);
            }
            // 紙のない所は透けないように、すべて不透明にする（三角形の外は使わない）。
            for px in rgba.chunks_exact_mut(4) {
                px[3] = 255;
            }
            rgba
        };
        Model { parts, textures: [texture(false), texture(true)], size: [w, h], edge_color: colors[2].0, roughness: [colors[0].1, colors[1].1, colors[2].1] }
    }

    /// GLB で保存する（場所を選ぶ）。
    pub(super) fn export_glb(&mut self) {
        let Some(path) = self.ask_output(t!("Save the 3D model (GLB)"), "GLB", "glb", t!("3d")) else { return };
        let result = glb(&self.model()).and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
        match result {
            Ok(()) => self.say(t!("Saved {}. It opens in Blender, in a browser viewer such as Babylon.js Sandbox, and in many 3D apps.", path.display())),
            Err(e) => self.say(t!("Could not save the 3D model: {e}")),
        }
    }

    /// OBJ で保存する（.obj と、同じ名前の .mtl と、表と裏の絵の PNG を、同じフォルダに）。
    pub(super) fn export_obj(&mut self) {
        let Some(path) = self.ask_output(t!("Save the 3D model (OBJ)"), "OBJ", "obj", t!("3d")) else { return };
        match obj(&self.model(), &path) {
            Ok(files) => self.say(t!("Saved {} files ({}). Keep them in the same folder.", files.len(), files.iter().filter_map(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", "))),
            Err(e) => self.say(t!("Could not save the 3D model: {e}")),
        }
    }
}

/// mm から m へ。
fn meters(p: [f32; 3]) -> [f32; 3] {
    p.map(|v| v * 0.001)
}

fn unit(n: [f32; 3]) -> [f32; 3] {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 1e-9 { n.map(|v| v / len) } else { [0.0, 1.0, 0.0] }
}

/// glTF 2.0 のバイナリ（.glb）を作る。三角形は頂点を共有しない形で並べる（索引なし）。
pub(super) fn glb(model: &Model) -> Result<Vec<u8>, String> {
    use serde_json::json;
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut push_view = |bin: &mut Vec<u8>, bytes: &[u8], target: Option<u32>| -> usize {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let offset = bin.len();
        bin.extend_from_slice(bytes);
        let mut view = json!({ "buffer": 0, "byteOffset": offset, "byteLength": bytes.len() });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        views.push(view);
        views.len() - 1
    };
    let mut meshes_primitives = Vec::new();
    for (part, tris) in model.parts.iter().enumerate() {
        if tris.is_empty() {
            continue;
        }
        let positions: Vec<[f32; 3]> = tris.iter().map(|v| meters(v.pos)).collect();
        let normals: Vec<[f32; 3]> = tris.iter().map(|v| unit(v.normal)).collect();
        let uvs: Vec<[f32; 2]> = tris.iter().map(|v| v.uv).collect();
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for p in &positions {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let floats = |data: &[f32]| data.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let pv = push_view(&mut bin, &floats(&positions.concat()), Some(34962));
        let nv = push_view(&mut bin, &floats(&normals.concat()), Some(34962));
        let tv = push_view(&mut bin, &floats(&uvs.concat()), Some(34962));
        let count = positions.len();
        accessors.push(json!({ "bufferView": pv, "componentType": 5126, "count": count, "type": "VEC3", "min": lo, "max": hi }));
        let pa = accessors.len() - 1;
        accessors.push(json!({ "bufferView": nv, "componentType": 5126, "count": count, "type": "VEC3" }));
        let na = accessors.len() - 1;
        accessors.push(json!({ "bufferView": tv, "componentType": 5126, "count": count, "type": "VEC2" }));
        let ta = accessors.len() - 1;
        meshes_primitives.push(json!({ "attributes": { "POSITION": pa, "NORMAL": na, "TEXCOORD_0": ta }, "material": part, "mode": 4 }));
    }
    // 表と裏の絵（PNG）。
    let mut images = Vec::new();
    for (i, rgba) in model.textures.iter().enumerate() {
        let png = super::outputs::png_bytes(model.size[0] as u32, model.size[1] as u32, rgba)?;
        let view = push_view(&mut bin, &png, None);
        images.push(json!({ "bufferView": view, "mimeType": "image/png", "name": PARTS[i] }));
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let srgb_to_linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let edge = model.edge_color.map(srgb_to_linear);
    // 紙は両面を別の三角形で持つので、片面だけ描けばよいが、ビューアによって向きの扱いが違うので両面にする。
    let materials = json!([
        { "name": "paper outside", "doubleSided": true, "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 }, "metallicFactor": 0.0, "roughnessFactor": model.roughness[0] } },
        { "name": "paper inside", "doubleSided": true, "pbrMetallicRoughness": { "baseColorTexture": { "index": 1 }, "metallicFactor": 0.0, "roughnessFactor": model.roughness[1] } },
        { "name": "paper edge", "doubleSided": true, "pbrMetallicRoughness": { "baseColorFactor": [edge[0], edge[1], edge[2], 1.0], "metallicFactor": 0.0, "roughnessFactor": model.roughness[2] } }
    ]);
    let gltf = json!({
        "asset": { "version": "2.0", "generator": format!("PAKENOBA {}", crate::VERSION_LABEL) },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0, "name": "box" }],
        "meshes": [{ "name": "box", "primitives": meshes_primitives }],
        "materials": materials,
        "textures": [{ "source": 0, "sampler": 0 }, { "source": 1, "sampler": 0 }],
        "samplers": [{ "magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071 }],
        "images": images,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{ "byteLength": bin.len() }]
    });
    let mut json_bytes = serde_json::to_vec(&gltf).map_err(|e| e.to_string())?;
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    Ok(out)
}

/// OBJ を書く。書いたファイルを返す。
pub(super) fn obj(model: &Model, path: &Path) -> Result<Vec<PathBuf>, String> {
    use std::fmt::Write;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "box".to_owned());
    let dir = path.parent().unwrap_or(Path::new("."));
    let mtl_name = format!("{stem}.mtl");
    let texture_names = [format!("{stem}_outside.png"), format!("{stem}_inside.png")];
    let mut text = format!("# PAKENOBA {} — box model in meters, +Y up\nmtllib {mtl_name}\n", crate::VERSION_LABEL);
    let mut next = 1usize;
    for (part, tris) in model.parts.iter().enumerate() {
        if tris.is_empty() {
            continue;
        }
        let _ = writeln!(text, "o {}\nusemtl {}", PARTS[part], PARTS[part]);
        for v in tris {
            let p = meters(v.pos);
            let n = unit(v.normal);
            // OBJ の絵の座標は、下が 0。
            let _ = writeln!(text, "v {:.6} {:.6} {:.6}\nvn {:.5} {:.5} {:.5}\nvt {:.6} {:.6}", p[0], p[1], p[2], n[0], n[1], n[2], v.uv[0], 1.0 - v.uv[1]);
        }
        for k in 0..tris.len() / 3 {
            let (a, b, c) = (next + 3 * k, next + 3 * k + 1, next + 3 * k + 2);
            let _ = writeln!(text, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}");
        }
        next += tris.len();
    }
    let e = model.edge_color.map(|c| c as f32 / 255.0);
    let mtl = format!(
        "# PAKENOBA\nnewmtl outside\nKd 1 1 1\nNs {:.0}\nmap_Kd {}\n\nnewmtl inside\nKd 1 1 1\nNs {:.0}\nmap_Kd {}\n\nnewmtl edge\nKd {:.4} {:.4} {:.4}\nNs {:.0}\n",
        (1.0 - model.roughness[0]) * 200.0,
        texture_names[0],
        (1.0 - model.roughness[1]) * 200.0,
        texture_names[1],
        e[0],
        e[1],
        e[2],
        (1.0 - model.roughness[2]) * 200.0
    );
    let files = [path.to_path_buf(), dir.join(&mtl_name), dir.join(&texture_names[0]), dir.join(&texture_names[1])];
    std::fs::write(&files[0], text).map_err(|e| e.to_string())?;
    std::fs::write(&files[1], mtl).map_err(|e| e.to_string())?;
    for i in 0..2 {
        let png = super::outputs::png_bytes(model.size[0] as u32, model.size[1] as u32, &model.textures[i])?;
        std::fs::write(&files[2 + i], png).map_err(|e| e.to_string())?;
    }
    Ok(files.to_vec())
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;

    #[test]
    fn glb_は決まりどおりの形で_三つの面と二枚の絵を持つ() {
        let app = App::new(&Initial::default());
        let model = app.model();
        assert!(model.parts.iter().all(|p| !p.is_empty() && p.len() % 3 == 0));
        let bytes = glb(&model).unwrap();
        assert_eq!(&bytes[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize, bytes.len());
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(&bytes[16..20], b"JSON");
        let gltf: serde_json::Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
        assert_eq!(gltf["meshes"][0]["primitives"].as_array().unwrap().len(), 3);
        assert_eq!(gltf["images"].as_array().unwrap().len(), 2);
        // 箱の大きさ（差し込み箱 200 × 160 × 85 mm）は、メートルで 0.2 前後。
        let max = gltf["accessors"][0]["max"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).fold(0.0f64, f64::max);
        assert!(max > 0.05 && max < 0.5, "{max}");
        let bin_start = 20 + json_len;
        assert_eq!(&bytes[bin_start + 4..bin_start + 8], b"BIN\0");
        assert_eq!(gltf["buffers"][0]["byteLength"].as_u64().unwrap() as usize, bytes.len() - bin_start - 8);
    }

    #[test]
    fn obj_は_mtl_と表裏の絵といっしょに書く() {
        let app = App::new(&Initial::default());
        let dir = std::env::temp_dir().join(format!("pakenoba-obj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let files = obj(&app.model(), &dir.join("箱.obj")).unwrap();
        assert_eq!(files.len(), 4);
        assert!(files.iter().all(|f| f.exists()));
        let text = std::fs::read_to_string(&files[0]).unwrap();
        assert!(text.contains("mtllib 箱.mtl") && text.contains("usemtl outside") && text.contains("\nf "));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
