//! 描画モード（Photoshop のレイヤーの描画モードと同じ計算）。
//!
//! 色は 0〜1。重ね方は W3C の Compositing and Blending と同じ形にする:
//! 下の絵がある所では、描画モードで混ぜた色を使い、下の絵がない所では、上の色をそのまま使ってから、ふつうに重ねる。

use pakenoba_core::project::Blend;

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - n).max(1e-6));
    }
    if x > 1.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l).max(1e-6));
    }
    c
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max - min <= 1e-6 {
        return [0.0; 3];
    }
    c.map(|v| (v - min) * s / (max - min))
}

/// 1 つの色の成分ごとに混ぜる描画モード。`b` は下（背景）、`s` は上（柄）。
fn separable(mode: Blend, b: f32, s: f32) -> f32 {
    match mode {
        Blend::Darken => b.min(s),
        Blend::Multiply => b * s,
        Blend::ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }
        Blend::LinearBurn => (b + s - 1.0).max(0.0),
        Blend::Lighten => b.max(s),
        Blend::Screen => b + s - b * s,
        Blend::ColorDodge => {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        Blend::LinearDodge => (b + s).min(1.0),
        Blend::Overlay => separable(Blend::HardLight, s, b),
        Blend::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        Blend::HardLight => {
            if s <= 0.5 {
                b * 2.0 * s
            } else {
                separable(Blend::Screen, b, 2.0 * s - 1.0)
            }
        }
        Blend::VividLight => {
            if s <= 0.5 {
                separable(Blend::ColorBurn, b, 2.0 * s)
            } else {
                separable(Blend::ColorDodge, b, 2.0 * s - 1.0)
            }
        }
        Blend::LinearLight => (b + 2.0 * s - 1.0).clamp(0.0, 1.0),
        Blend::PinLight => {
            if s <= 0.5 {
                b.min(2.0 * s)
            } else {
                b.max(2.0 * s - 1.0)
            }
        }
        Blend::HardMix => {
            if separable(Blend::VividLight, b, s) < 0.5 {
                0.0
            } else {
                1.0
            }
        }
        Blend::Difference => (b - s).abs(),
        Blend::Exclusion => b + s - 2.0 * b * s,
        Blend::Subtract => (b - s).max(0.0),
        Blend::Divide => {
            if s <= 0.0 {
                if b > 0.0 { 1.0 } else { 0.0 }
            } else {
                (b / s).min(1.0)
            }
        }
        _ => s,
    }
}

/// 下の色 `b` と上の色 `s`（どちらも 0〜1）を、描画モードで混ぜた色。
pub fn mix(mode: Blend, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        Blend::Normal => s,
        Blend::DarkerColor => {
            if lum(s) < lum(b) {
                s
            } else {
                b
            }
        }
        Blend::LighterColor => {
            if lum(s) > lum(b) {
                s
            } else {
                b
            }
        }
        Blend::Hue => set_lum(set_sat(s, sat(b)), lum(b)),
        Blend::Saturation => set_lum(set_sat(b, sat(s)), lum(b)),
        Blend::Color => set_lum(s, lum(b)),
        Blend::Luminosity => set_lum(b, lum(s)),
        _ => [separable(mode, b[0], s[0]), separable(mode, b[1], s[1]), separable(mode, b[2], s[2])],
    }
}

/// RGBA（8 bit、アルファは乗算なし）の画素 `dst` に、色 `src`（0〜255）、不透明度 `alpha`（0〜1）を、描画モードで重ねる。
pub fn over(dst: &mut [u8], src: [f32; 3], alpha: f32, mode: Blend) {
    if alpha <= 0.0 {
        return;
    }
    let below = dst[3] as f32 / 255.0;
    let s = src.map(|v| v / 255.0);
    let color = if mode == Blend::Normal || below <= 0.0 {
        s
    } else {
        let b = [dst[0] as f32 / 255.0, dst[1] as f32 / 255.0, dst[2] as f32 / 255.0];
        let m = mix(mode, b, s);
        // 下の絵が半ば透けている所では、混ぜた色と上の色を、その割合で合わせる。
        [0, 1, 2].map(|i| (1.0 - below) * s[i] + below * m[i])
    };
    let out_a = alpha + below * (1.0 - alpha);
    for i in 0..3 {
        let b = dst[i] as f32 / 255.0;
        dst[i] = (((color[i] * alpha + b * below * (1.0 - alpha)) / out_a).clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    dst[3] = (out_a * 255.0).round() as u8;
}

/// 案件の描画モードを、PSD の描画モードへ。
pub fn psd(mode: Blend) -> craft_psd::BlendMode {
    use craft_psd::BlendMode as P;
    match mode {
        Blend::Normal => P::Normal,
        Blend::Darken => P::Darken,
        Blend::Multiply => P::Multiply,
        Blend::ColorBurn => P::ColorBurn,
        Blend::LinearBurn => P::LinearBurn,
        Blend::DarkerColor => P::DarkerColor,
        Blend::Lighten => P::Lighten,
        Blend::Screen => P::Screen,
        Blend::ColorDodge => P::ColorDodge,
        Blend::LinearDodge => P::LinearDodge,
        Blend::LighterColor => P::LighterColor,
        Blend::Overlay => P::Overlay,
        Blend::SoftLight => P::SoftLight,
        Blend::HardLight => P::HardLight,
        Blend::VividLight => P::VividLight,
        Blend::LinearLight => P::LinearLight,
        Blend::PinLight => P::PinLight,
        Blend::HardMix => P::HardMix,
        Blend::Difference => P::Difference,
        Blend::Exclusion => P::Exclusion,
        Blend::Subtract => P::Subtract,
        Blend::Divide => P::Divide,
        Blend::Hue => P::Hue,
        Blend::Saturation => P::Saturation,
        Blend::Color => P::Color,
        Blend::Luminosity => P::Luminosity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 代表的な描画モードの計算() {
        let b = [0.5, 0.2, 0.8];
        let s = [0.5, 0.5, 0.5];
        assert_eq!(mix(Blend::Multiply, b, s), [0.25, 0.1, 0.4]);
        assert_eq!(mix(Blend::Screen, [0.5; 3], [0.5; 3]), [0.75; 3]);
        // 50% の灰色は、オーバーレイ、ソフトライト、ハードライトで、下の色を変えない。
        for mode in [Blend::Overlay, Blend::SoftLight, Blend::HardLight] {
            let out = mix(mode, b, s);
            assert!(out.iter().zip(b).all(|(o, b)| (o - b).abs() < 1e-5), "{mode:?} {out:?}");
        }
        assert_eq!(mix(Blend::Difference, [1.0, 0.0, 0.5], [0.25, 0.25, 0.5]), [0.75, 0.25, 0.0]);
        // 輝度は、上の明るさと下の色み。カラーは、その逆。
        let l = mix(Blend::Luminosity, [1.0, 0.0, 0.0], [0.5, 0.5, 0.5]);
        assert!((lum(l) - 0.5).abs() < 1e-4 && l[0] > l[1]);
        let c = mix(Blend::Color, [0.5, 0.5, 0.5], [1.0, 0.0, 0.0]);
        assert!((lum(c) - 0.5).abs() < 1e-4 && c[0] > c[1]);
    }

    #[test]
    fn 重ね方() {
        // 不透明な白の上に、半分の不透明度の黒を乗算。
        let mut px = [255, 255, 255, 255];
        over(&mut px, [0.0; 3], 0.5, Blend::Multiply);
        assert_eq!(px, [128, 128, 128, 255]);
        // 透明な所には、描画モードによらず、上の色がそのまま乗る。
        let mut px = [0, 0, 0, 0];
        over(&mut px, [200.0, 100.0, 50.0], 1.0, Blend::Multiply);
        assert_eq!(px, [200, 100, 50, 255]);
        // 白の上のスクリーンは、白のまま。
        let mut px = [255, 255, 255, 255];
        over(&mut px, [10.0, 20.0, 30.0], 1.0, Blend::Screen);
        assert_eq!(px, [255, 255, 255, 255]);
    }

    #[test]
    fn どの描画モードも_psd_の名前がある() {
        for mode in Blend::MENU.iter().flatten().filter(|m| **m != Blend::Normal) {
            assert_ne!(psd(*mode), craft_psd::BlendMode::Normal, "{mode:?}");
        }
    }
}
