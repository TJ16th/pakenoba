//! PSD の読み書き（craft-psd）。

use craft_psd::{LayerSpec, PixelData, PsdBuilder, PsdFile};

use crate::raster::Texture;
use crate::scene::BoxDims;

/// 読み込んだ PSD。
pub struct LoadedPsd {
    pub texture: Texture,
    pub layer_names: Vec<String>,
    /// PSD に本物の統合画像が入っているか。false のときは白い仮画像しか入っていない
    /// （Photoshop の「互換性を優先」を切って保存したファイルなど）。
    pub has_real_merged: Option<bool>,
}

/// PSD のバイト列から、統合画像とレイヤー名を取り出す。
pub fn load_psd(bytes: &[u8]) -> Result<LoadedPsd, String> {
    let file = PsdFile::from_bytes(bytes).map_err(|e| format!("PSD を読めませんでした: {e}"))?;
    let image = file.composite_rgba8().map_err(|e| format!("統合画像を取り出せませんでした: {e}"))?;
    let (w, h) = (image.width as usize, image.height as usize);
    if image.data.len() != w * h * 4 {
        return Err(format!("統合画像の大きさが合いません（{w}x{h} に対して {} バイト）", image.data.len()));
    }
    let layer_names = file.iter_layers().map(|l| l.name()).collect();
    Ok(LoadedPsd { texture: Texture { w, h, rgba: image.data }, layer_names, has_real_merged: file.has_real_merged_data() })
}

/// 確認用の PSD を作る。レイヤー2枚（日本語名）と統合画像を持つ。
///
/// 絵柄は「正面から右側面へ続く帯」の大きさで作り、面の境目をまたぐ円を置く。
pub fn make_sample_psd(dims: BoxDims) -> Result<Vec<u8>, String> {
    const PX_PER_MM: f32 = 8.0;
    let w = ((dims.w + dims.d) * PX_PER_MM).round().max(16.0) as u32;
    let h = (dims.h * PX_PER_MM).round().max(16.0) as u32;
    let split_x = dims.w * PX_PER_MM;
    let (wu, hu) = (w as usize, h as usize);

    let mut background = vec![0u8; wu * hu * 4];
    let mut pattern = vec![0u8; wu * hu * 4];
    let mut composite = vec![0u8; wu * hu * 4];

    let circle_r = dims.h.min(dims.d * 2.0) * PX_PER_MM * 0.32;
    let circle_c = (split_x, h as f32 * 0.45);
    let marker_c = (w as f32 * 0.07, h as f32 * 0.08);
    let marker_r = h as f32 * 0.035;

    for y in 0..hu {
        let t = y as f32 / (hu - 1) as f32;
        let bg = [(238.0 - 26.0 * t) as u8, (104.0 - 22.0 * t) as u8, (148.0 - 18.0 * t) as u8];
        for x in 0..wu {
            let o = (y * wu + x) * 4;
            background[o..o + 4].copy_from_slice(&[bg[0], bg[1], bg[2], 255]);

            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // 模様レイヤー: 斜めの縞、境目をまたぐ円、左上の目印。
            let mut px = [255u8, 255, 255, 0];
            let stripe = ((fx + fy) / (PX_PER_MM * 9.0)).rem_euclid(1.0);
            if stripe < 0.28 {
                px[3] = 46;
            }
            let dc = ((fx - circle_c.0).powi(2) + (fy - circle_c.1).powi(2)).sqrt();
            if dc < circle_r {
                px = [255, 252, 245, 255];
            } else if dc < circle_r * 1.12 {
                px = [120, 20, 60, 255];
            }
            let dm = ((fx - marker_c.0).powi(2) + (fy - marker_c.1).powi(2)).sqrt();
            if dm < marker_r {
                px = [40, 30, 50, 255];
            }
            pattern[o..o + 4].copy_from_slice(&px);

            let a = px[3] as f32 / 255.0;
            for i in 0..3 {
                composite[o + i] = (px[i] as f32 * a + bg[i] as f32 * (1.0 - a)).round() as u8;
            }
            composite[o + 3] = 255;
        }
    }

    let mut builder = PsdBuilder::new(w, h);
    builder.push_layer(LayerSpec::new("背景", 0, 0, w, h, PixelData::Rgba8(background)));
    builder.push_layer(LayerSpec::new("模様", 0, 0, w, h, PixelData::Rgba8(pattern)));
    builder.composite(PixelData::Rgba8(composite));
    builder.to_bytes().map_err(|e| format!("PSD を書けませんでした: {e}"))
}
