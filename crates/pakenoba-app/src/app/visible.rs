//! 柄の、見えている部分の範囲（透明な余白を除いた四角）。
//!
//! 選んだ柄の枠とつまみ、クリックで拾う範囲、吸い付き、カーソルの下の枠は、この範囲に合わせる
//! （Photoshop の変形の枠、Illustrator の選択枠と同じ）。プロパティの数値は、柄の大きさのまま。
//!
//! 範囲は、柄の座標の割合で持つ（左下が -1、右上が 1）。見せ方（ベクターの色ごとの表示、調整）を効かせた絵で測る。

use std::cell::RefCell;
use std::collections::HashMap;

use pakenoba_core::surface::Decal;
use pakenoba_core::template::{P2, Template};

use crate::paint::{Picture, Placed};

/// 余白のない柄（いっぱいの四角）。
pub(crate) const FULL: [f32; 4] = [-1.0, -1.0, 1.0, 1.0];

/// 見えているとみなす不透明度（これより薄い画素は余白）。
const ALPHA: u8 = 8;

thread_local! {
    /// 絵ごと（中身の場所と大きさ）の範囲。
    static CACHE: RefCell<HashMap<(usize, usize, usize), [f32; 4]>> = RefCell::new(HashMap::new());
}

/// 絵の、見えている部分の範囲。全部が透明なら、いっぱいの四角にする（つかめなくならないように）。
pub(crate) fn of_picture(picture: &Picture) -> [f32; 4] {
    let Picture::Image { w, h, rgba, .. } = picture else { return FULL };
    let (w, h) = (*w, *h);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return FULL;
    }
    let key = (std::sync::Arc::as_ptr(rgba) as usize, w, h);
    if let Some(found) = CACHE.with(|c| c.borrow().get(&key).copied()) {
        return found;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        let row = &rgba[y * w * 4..(y + 1) * w * 4];
        let mut first = None;
        let mut last = 0;
        for x in 0..w {
            if row[x * 4 + 3] >= ALPHA {
                first.get_or_insert(x);
                last = x;
            }
        }
        if let Some(first) = first {
            x0 = x0.min(first);
            x1 = x1.max(last + 1);
            y0 = y0.min(y);
            y1 = y1.max(y + 1);
        }
    }
    let area = if x1 <= x0 || y1 <= y0 {
        FULL
    } else {
        // 画素の行は上から下、柄の座標は下が -1。
        let fx = |x: usize| x as f32 / w as f32 * 2.0 - 1.0;
        let fy = |y: usize| 1.0 - y as f32 / h as f32 * 2.0;
        [fx(x0), fy(y1), fx(x1), fy(y0)]
    };
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 512 {
            c.clear();
        }
        c.insert(key, area);
    });
    area
}

/// 置いた柄の、見えている部分の範囲（見せ方を効かせた絵で測る）。
pub(crate) fn of(placed: &Placed) -> [f32; 4] {
    of_picture(&crate::effects::shown(placed, false))
}

/// 柄を、見えている部分だけの大きさにした写しと、その中心の、もとの柄の座標での位置（mm）。
pub(crate) fn trim(decal: &Decal, tpl: &Template, area: [f32; 4]) -> (Decal, P2) {
    if area == FULL {
        return (*decal, [0.0, 0.0]);
    }
    let half = [decal.size[0] * 0.5, decal.size[1] * 0.5];
    let offset = [(area[0] + area[2]) * 0.5 * half[0], (area[1] + area[3]) * 0.5 * half[1]];
    let mut out = *decal;
    out.size = [((area[2] - area[0]) * half[0]).max(0.5), ((area[3] - area[1]) * half[1]).max(0.5)];
    out.set_center(tpl, decal.place(tpl).point(offset));
    (out, offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn margins_are_left_out() {
        // 10 x 10 の絵の、右上の 4 x 2 だけが見える。
        let mut rgba = vec![0u8; 10 * 10 * 4];
        for y in 1..3 {
            for x in 5..9 {
                rgba[(y * 10 + x) * 4 + 3] = 255;
            }
        }
        let area = of_picture(&Picture::Image { w: 10, h: 10, rgba: Arc::new(rgba), vector: false });
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(area[0], 0.0) && close(area[2], 0.8) && close(area[1], 0.4) && close(area[3], 0.8), "{area:?}");
        // 全部が透明なら、いっぱい。
        let empty = of_picture(&Picture::Image { w: 4, h: 4, rgba: Arc::new(vec![0; 64]), vector: false });
        assert_eq!(empty, FULL);
    }
}
