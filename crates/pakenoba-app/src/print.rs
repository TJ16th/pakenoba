//! 実寸の型紙を、A4 や A3 に分けて刷る PDF。
//!
//! 展開図を実寸（100%）で、紙に入る大きさに区切って並べる。隣の紙とは「重ねしろ」のぶんだけ重ねて刷り、
//! 重ねて貼り合わせられるようにする。切り線と折り線は、拡大しても滲まないよう、線（ベクター）で描く。
//! 柄の絵は、展開図の画像を 1 回だけ埋め込み、どの紙からも同じものを使う（ファイルを小さくするため）。
//! 紙ごとに、場所（行と列）と、何枚目かを、左上に小さく刷る。1 枚目には、刷った大きさを確かめる 100 mm の目盛りを刷る。

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str};

use pakenoba_core::template::P2;

/// mm を、PDF の単位（1/72 インチ）に直す。
const PT_PER_MM: f32 = 72.0 / 25.4;

/// 刷る紙。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paper {
    A4,
    A3,
}

impl Paper {
    /// 縦長の向きでの、幅と高さ（mm）。
    pub fn size(self) -> [f32; 2] {
        match self {
            Paper::A4 => [210.0, 297.0],
            Paper::A3 => [297.0, 420.0],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::A3 => "A3",
        }
    }
}

/// 刷り方。
#[derive(Clone, Copy, Debug)]
pub struct PrintSettings {
    pub paper: Paper,
    /// 紙の端の、刷らない幅（mm）。家庭用のプリンターの多くは、端の数 mm を刷れない。
    pub margin: f32,
    /// 隣の紙と重ねる幅（mm）。
    pub overlap: f32,
}

impl Default for PrintSettings {
    fn default() -> Self {
        PrintSettings { paper: Paper::A4, margin: 10.0, overlap: 10.0 }
    }
}

/// 紙の分け方。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tiling {
    /// 紙の幅と高さ（mm。向きを決めたあと）。
    pub page: [f32; 2],
    pub columns: usize,
    pub rows: usize,
    /// 隣の紙へ進む幅（mm。刷れる幅から、重ねしろを引いたもの）。
    pub step: [f32; 2],
}

impl Tiling {
    pub fn pages(&self) -> usize {
        self.columns * self.rows
    }

    pub fn landscape(&self) -> bool {
        self.page[0] > self.page[1]
    }
}

/// 展開図（幅と高さ、mm）を、紙に分ける。縦向きと横向きのうち、枚数の少ないほう（同じなら縦向き）。
pub fn tile(sheet: [f32; 2], settings: &PrintSettings) -> Tiling {
    let [w, h] = settings.paper.size();
    let fit = |page: [f32; 2]| {
        let usable = [page[0] - settings.margin * 2.0, page[1] - settings.margin * 2.0];
        let step = [(usable[0] - settings.overlap).max(10.0), (usable[1] - settings.overlap).max(10.0)];
        let count = |len: f32, step: f32| (((len - settings.overlap).max(0.0)) / step).ceil().max(1.0) as usize;
        Tiling { page, columns: count(sheet[0], step[0]), rows: count(sheet[1], step[1]), step }
    };
    let (portrait, landscape) = (fit([w, h]), fit([h, w]));
    if landscape.pages() < portrait.pages() { landscape } else { portrait }
}

/// 刷る紙（行と列）。展開図の紙がまったくかからない紙は、刷らない。
/// `alpha` は展開図の形（アルファ。左上から、幅 `w`、高さ `h`、1 mm あたり `w / 展開図の幅` 画素）。
pub fn used_tiles(tiling: &Tiling, size: [f32; 2], margin: f32, w: usize, h: usize, alpha: impl Fn(usize, usize) -> u8) -> Vec<(usize, usize)> {
    let usable = [tiling.page[0] - 2.0 * margin, tiling.page[1] - 2.0 * margin];
    let (sx, sy) = (w as f32 / size[0], h as f32 / size[1]);
    let mut used = Vec::new();
    for row in 0..tiling.rows {
        for column in 0..tiling.columns {
            let (x0, y0) = (column as f32 * tiling.step[0], row as f32 * tiling.step[1]);
            let px = |x: f32| ((x * sx).max(0.0) as usize).min(w);
            let py = |y: f32| ((y * sy).max(0.0) as usize).min(h);
            let (a, b, c, d) = (px(x0), px(x0 + usable[0]), py(y0), py(y0 + usable[1]));
            let step = 2;
            let touched = (c..d).step_by(step).any(|y| (a..b).step_by(step).any(|x| alpha(x, y) > 0));
            if touched {
                used.push((row, column));
            }
        }
    }
    used
}

/// 展開図の絵と線。座標は展開図の mm（左下が `min`、上が正）。
pub struct SheetPrint<'a> {
    pub min: P2,
    pub max: P2,
    /// 柄と紙の色の絵（RGBA、左上から。ガイドの線は入れない）。アルファが 0 のところは紙の外。
    pub w: usize,
    pub h: usize,
    pub rgba: &'a [u8],
    /// 切り線と、折り線。
    pub cuts: &'a [(P2, P2)],
    pub folds: &'a [(P2, P2)],
    /// 1 枚目に刷る、案件の名前（英数字以外は刷れないので、刷るのは英数字の部分だけ）。
    pub title: &'a str,
}

/// PDF の文字は、標準のフォント（Helvetica）で刷るので、英数字と記号だけにする。
fn ascii(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).collect()
}

/// 実寸の型紙の PDF を作る。
pub fn print_pdf(sheet: &SheetPrint, settings: &PrintSettings) -> Result<Vec<u8>, String> {
    let size = [sheet.max[0] - sheet.min[0], sheet.max[1] - sheet.min[1]];
    let tiling = tile(size, settings);
    let mut pdf = Pdf::new();
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let image = Ref::new(3);
    let font = Ref::new(4);
    let mut next = 5;
    let mut alloc = || {
        let r = Ref::new(next);
        next += 1;
        r
    };

    // 絵は、白い紙の上に重ねた RGB にして、圧縮して 1 回だけ入れる。
    let rgb: Vec<u8> = sheet
        .rgba
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as u32;
            [0, 1, 2].map(|i| ((p[i] as u32 * a + 255 * (255 - a) + 127) / 255) as u8)
        })
        .collect();
    let mut zip = ZlibEncoder::new(Vec::new(), Compression::default());
    zip.write_all(&rgb).map_err(|e| e.to_string())?;
    let compressed = zip.finish().map_err(|e| e.to_string())?;
    let mut xobject = pdf.image_xobject(image, &compressed);
    xobject.filter(Filter::FlateDecode);
    xobject.width(sheet.w as i32);
    xobject.height(sheet.h as i32);
    xobject.color_space().device_rgb();
    xobject.bits_per_component(8);
    xobject.finish();
    pdf.type1_font(font).base_font(Name(b"Helvetica"));

    let used = used_tiles(&tiling, size, settings.margin, sheet.w, sheet.h, |x, y| sheet.rgba[(y * sheet.w + x) * 4 + 3]);
    let pages = used.len();
    let mut page_ids = Vec::new();
    let k = PT_PER_MM;
    let [pw, ph] = tiling.page;
    let m = settings.margin;
    for (number, &(row, column)) in used.iter().enumerate() {
        let number = number + 1;
        {
            let page_id = alloc();
            let content_id = alloc();
            page_ids.push(page_id);
            // この紙に刷る、展開図の範囲の左上（mm）。
            let (x0, top) = (sheet.min[0] + column as f32 * tiling.step[0], sheet.max[1] - row as f32 * tiling.step[1]);
            // 展開図の点 (x, y) を、紙の上の点（mm。左下が原点）へ。
            let to_page = |p: P2| [m + (p[0] - x0), ph - m - (top - p[1])];
            let mut content = Content::new();
            content.save_state();
            // 刷れる範囲で切り抜く。
            content.rect(m * k, m * k, (pw - 2.0 * m) * k, (ph - 2.0 * m) * k);
            content.clip_nonzero();
            content.end_path();
            // 絵: 1 x 1 の画像を、展開図の大きさに引き伸ばして置く。
            let corner = to_page([sheet.min[0], sheet.min[1]]);
            content.save_state();
            content.transform([size[0] * k, 0.0, 0.0, size[1] * k, corner[0] * k, corner[1] * k]);
            content.x_object(Name(b"Im0"));
            content.restore_state();
            // 切り線（黒、実線）と折り線（青、破線）。
            content.set_line_width(0.6);
            content.set_stroke_rgb(0.1, 0.1, 0.12);
            for &(a, b) in sheet.cuts {
                let (a, b) = (to_page(a), to_page(b));
                content.move_to(a[0] * k, a[1] * k);
                content.line_to(b[0] * k, b[1] * k);
            }
            content.stroke();
            content.set_stroke_rgb(0.0, 0.55, 0.85);
            content.set_dash_pattern([3.0, 2.0], 0.0);
            for &(a, b) in sheet.folds {
                let (a, b) = (to_page(a), to_page(b));
                content.move_to(a[0] * k, a[1] * k);
                content.line_to(b[0] * k, b[1] * k);
            }
            content.stroke();
            content.restore_state();

            // 重ねしろの印（薄い灰色の点線）。右と下の隣の紙と重なる帯の、内側の端。
            content.save_state();
            content.set_line_width(0.4);
            content.set_stroke_rgb(0.6, 0.6, 0.6);
            content.set_dash_pattern([1.5, 1.5], 0.0);
            if column + 1 < tiling.columns {
                let x = (pw - m - settings.overlap) * k;
                content.move_to(x, m * k);
                content.line_to(x, (ph - m) * k);
            }
            if row + 1 < tiling.rows {
                let y = (m + settings.overlap) * k;
                content.move_to(m * k, y);
                content.line_to((pw - m) * k, y);
            }
            content.stroke();
            // 刷れる範囲の角の印（貼り合わせるときの目安）。
            content.set_dash_pattern([], 0.0);
            content.set_stroke_rgb(0.3, 0.3, 0.3);
            for (cx, cy, sx, sy) in [(m, m, 1.0, 1.0), (pw - m, m, -1.0, 1.0), (m, ph - m, 1.0, -1.0), (pw - m, ph - m, -1.0, -1.0)] {
                content.move_to(cx * k, (cy + sy * 4.0) * k);
                content.line_to(cx * k, cy * k);
                content.line_to((cx + sx * 4.0) * k, cy * k);
            }
            content.stroke();
            content.restore_state();

            // 紙の場所と、何枚目か。
            let label = format!("Pakenoba  {}  row {} / col {}   page {number} of {pages}   print at 100% (no scaling)", ascii(sheet.title), row + 1, column + 1);
            content.begin_text();
            content.set_font(Name(b"F1"), 7.0);
            content.set_fill_gray(0.35);
            content.next_line(m * k, (ph - m + 3.0) * k);
            content.show(Str(label.as_bytes()));
            content.end_text();
            if number == 1 {
                // 100 mm の目盛り。刷ったあとに定規で測って、実寸で刷れたかを確かめる。
                let (bx, by) = (m, m * 0.35);
                content.save_state();
                content.set_line_width(0.5);
                content.set_stroke_gray(0.2);
                content.move_to(bx * k, by * k);
                content.line_to((bx + 100.0) * k, by * k);
                for t in 0..=10 {
                    let x = (bx + t as f32 * 10.0) * k;
                    let tall = if t % 5 == 0 { 2.0 } else { 1.0 };
                    content.move_to(x, by * k);
                    content.line_to(x, (by + tall) * k);
                }
                content.stroke();
                content.restore_state();
                content.begin_text();
                content.set_font(Name(b"F1"), 6.0);
                content.set_fill_gray(0.35);
                content.next_line((bx + 102.0) * k, by * k);
                content.show(Str(b"100 mm"));
                content.end_text();
            }

            let mut page = pdf.page(page_id);
            page.media_box(Rect::new(0.0, 0.0, pw * k, ph * k));
            page.parent(tree);
            page.contents(content_id);
            let mut resources = page.resources();
            resources.x_objects().pair(Name(b"Im0"), image);
            resources.fonts().pair(Name(b"F1"), font);
            resources.finish();
            page.finish();
            pdf.stream(content_id, &content.finish());
        }
    }
    pdf.pages(tree).kids(page_ids.iter().copied()).count(page_ids.len() as i32);
    pdf.catalog(catalog).pages(tree);
    Ok(pdf.finish())
}

/// 入稿用の PDF の、紙の面 1 つ（表か裏）。座標は、展開図の mm（`print_ready_pdf` に渡す `min`、`max` の範囲）。
pub struct ReadySide<'a> {
    /// 面の名前（"Front" や "Back"。英数字だけ刷る）。
    pub label: &'a str,
    /// 刷る絵（RGBA、左上から、展開図の大きさ）。紙の色は入れない（アルファが 0 の所は、刷らない）。
    pub rgba: &'a [u8],
    /// 加工の版（特色）。`ink` が 128 以上の所が版。
    pub plates: &'a [crate::art::Plate],
    pub cuts: &'a [(P2, P2)],
    pub folds: &'a [(P2, P2)],
}

/// 入稿用の PDF の、紙の端の余白（mm）。面の名前を刷る。
const READY_MARGIN: f32 = 15.0;

/// 版の、代わりに見せる CMYK（特色を刷れない機械や、画面で見るときの色）。画面の色から作り、白は薄い水色にする。
fn plate_cmyk(rgb: [u8; 3]) -> [f32; 4] {
    let [r, g, b] = rgb.map(|v| v as f32 / 255.0);
    let k = 1.0 - r.max(g).max(b);
    if k >= 0.999 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let cmyk = [(1.0 - r - k) / (1.0 - k), (1.0 - g - k) / (1.0 - k), (1.0 - b - k) / (1.0 - k), k];
    if cmyk.iter().all(|&v| v < 0.05) { [0.15, 0.0, 0.0, 0.0] } else { cmyk }
}

/// 入稿用の PDF を作る。面ごとに 1 ページ（展開図の実寸に、余白を足した大きさ）。
///
/// - 絵は、白の上に重ねた RGB の画像。
/// - 切り線は特色「Die Cut」、折り線は特色「Crease」の線（ベクター、オーバープリント）。
/// - 加工の版は、版の名前の特色（Separation）で、オーバープリントにして、絵の上に重ねる。
///   版の形は 1 ビットのステンシル（画像マスク）なので、特色を見せない画面では、代わりの CMYK で絵の上に見える。
/// - 最後のページに、加工の指示（`notes`。英数字だけ刷る）。
pub fn print_ready_pdf(min: P2, max: P2, w: usize, h: usize, sides: &[ReadySide], title: &str, notes: &[String]) -> Result<Vec<u8>, String> {
    let size = [max[0] - min[0], max[1] - min[1]];
    let k = PT_PER_MM;
    let m = READY_MARGIN;
    let (pw, ph) = (size[0] + 2.0 * m, size[1] + 2.0 * m);
    let mut pdf = Pdf::new();
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let font = Ref::new(3);
    let overprint = Ref::new(4);
    let die = Ref::new(5);
    let crease = Ref::new(6);
    let mut next = 7;
    let mut alloc = || {
        let r = Ref::new(next);
        next += 1;
        r
    };
    pdf.type1_font(font).base_font(Name(b"Helvetica"));
    pdf.ext_graphics(overprint).overprint(true).overprint_fill(true).overprint_mode(pdf_writer::types::OverprintMode::IgnoreZeroChannel);
    let separation = |pdf: &mut Pdf, id: Ref, name: &str, cmyk: [f32; 4]| {
        let mut space = pdf.color_space(id).separation(Name(name.as_bytes()));
        space.alternate_color_space().device_cmyk();
        space.tint_exponential().domain([0.0, 1.0]).c0([0.0; 4]).c1(cmyk).n(1.0);
    };
    separation(&mut pdf, die, "Die Cut", [0.0, 1.0, 0.0, 0.0]);
    separation(&mut pdf, crease, "Crease", [1.0, 0.0, 0.0, 0.0]);
    let deflate = |data: &[u8]| -> Result<Vec<u8>, String> {
        let mut zip = ZlibEncoder::new(Vec::new(), Compression::default());
        zip.write_all(data).map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())
    };
    // 同じ名前の特色は、どの面でも同じ色空間を使う。
    let mut spaces: Vec<(String, Ref)> = Vec::new();
    let mut page_ids = Vec::new();
    for side in sides {
        let page_id = alloc();
        let content_id = alloc();
        let image = alloc();
        page_ids.push(page_id);
        let rgb: Vec<u8> = side
            .rgba
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3] as u32;
                [0, 1, 2].map(|i| ((p[i] as u32 * a + 255 * (255 - a) + 127) / 255) as u8)
            })
            .collect();
        let compressed = deflate(&rgb)?;
        let mut xobject = pdf.image_xobject(image, &compressed);
        xobject.filter(Filter::FlateDecode);
        xobject.width(w as i32).height(h as i32).bits_per_component(8);
        xobject.color_space().device_rgb();
        xobject.finish();
        let mut masks = Vec::new();
        for plate in side.plates {
            let space = match spaces.iter().find(|(n, _)| *n == plate.name) {
                Some(&(_, r)) => r,
                None => {
                    let r = alloc();
                    separation(&mut pdf, r, &plate.name, plate_cmyk(plate.rgb));
                    spaces.push((plate.name.clone(), r));
                    r
                }
            };
            // 1 ビットの画像マスク。0 の所に塗る。行はバイトの切れ目でそろえる。
            let row = w.div_ceil(8);
            let mut bits = vec![0xffu8; row * h];
            for y in 0..h {
                for x in 0..w {
                    if plate.ink[y * w + x] >= 128 {
                        bits[y * row + x / 8] &= !(0x80 >> (x % 8));
                    }
                }
            }
            let mask = alloc();
            let compressed = deflate(&bits)?;
            let mut xobject = pdf.image_xobject(mask, &compressed);
            xobject.filter(Filter::FlateDecode);
            xobject.width(w as i32).height(h as i32).image_mask(true);
            xobject.finish();
            masks.push((space, mask));
        }

        let to_page = |p: P2| [(m + p[0] - min[0]) * k, (m + p[1] - min[1]) * k];
        let mut content = Content::new();
        content.save_state();
        content.transform([size[0] * k, 0.0, 0.0, size[1] * k, m * k, m * k]);
        content.x_object(Name(b"Im0"));
        content.restore_state();
        content.save_state();
        content.set_parameters(Name(b"GS0"));
        for (i, _) in masks.iter().enumerate() {
            let (cs, mk) = (format!("CS{i}"), format!("Mk{i}"));
            content.save_state();
            content.set_fill_color_space(Name(cs.as_bytes()));
            content.set_fill_color([1.0]);
            content.transform([size[0] * k, 0.0, 0.0, size[1] * k, m * k, m * k]);
            content.x_object(Name(mk.as_bytes()));
            content.restore_state();
        }
        content.set_line_width(0.25 * k);
        for (name, lines, dash) in [(b"CSDie".as_slice(), side.cuts, false), (b"CSCrease".as_slice(), side.folds, true)] {
            content.set_stroke_color_space(Name(name));
            content.set_stroke_color([1.0]);
            if dash {
                content.set_dash_pattern([3.0 * k, 1.5 * k], 0.0);
            }
            for &(a, b) in lines {
                let (a, b) = (to_page(a), to_page(b));
                content.move_to(a[0], a[1]);
                content.line_to(b[0], b[1]);
            }
            content.stroke();
        }
        content.restore_state();
        let label = format!("PAKENOBA print-ready  {}  {}  actual size  ({:.1} x {:.1} mm)  plates: {}", ascii(title), ascii(side.label), size[0], size[1], side.plates.iter().map(|p| ascii(&p.name)).collect::<Vec<_>>().join(", "));
        content.begin_text();
        content.set_font(Name(b"F1"), 7.0);
        content.set_fill_gray(0.35);
        content.next_line(m * k, (ph - m * 0.6) * k);
        content.show(Str(label.as_bytes()));
        content.end_text();

        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, pw * k, ph * k));
        page.trim_box(Rect::new(m * k, m * k, (pw - m) * k, (ph - m) * k));
        page.parent(tree);
        page.contents(content_id);
        let mut resources = page.resources();
        let mut objects = resources.x_objects();
        objects.pair(Name(b"Im0"), image);
        for (i, &(_, mask)) in masks.iter().enumerate() {
            objects.pair(Name(format!("Mk{i}").as_bytes()), mask);
        }
        objects.finish();
        let mut colors = resources.color_spaces();
        colors.pair(Name(b"CSDie"), die);
        colors.pair(Name(b"CSCrease"), crease);
        for (i, &(space, _)) in masks.iter().enumerate() {
            colors.pair(Name(format!("CS{i}").as_bytes()), space);
        }
        colors.finish();
        resources.ext_g_states().pair(Name(b"GS0"), overprint);
        resources.fonts().pair(Name(b"F1"), font);
        resources.finish();
        page.finish();
        pdf.stream(content_id, &content.finish());
    }

    // 指示のページ（A4 縦）。
    let page_id = alloc();
    let content_id = alloc();
    page_ids.push(page_id);
    let mut content = Content::new();
    content.begin_text();
    content.set_font(Name(b"F1"), 14.0);
    content.set_fill_gray(0.0);
    content.next_line(20.0 * k, 275.0 * k);
    content.show(Str(format!("Finishing instructions  {}", ascii(title)).as_bytes()));
    content.set_font(Name(b"F1"), 10.0);
    content.set_leading(14.0);
    content.next_line(0.0, -24.0);
    for line in notes {
        content.show(Str(ascii(line).as_bytes()));
        content.next_line_using_leading();
    }
    content.end_text();
    let mut page = pdf.page(page_id);
    page.media_box(Rect::new(0.0, 0.0, 210.0 * k, 297.0 * k));
    page.parent(tree);
    page.contents(content_id);
    page.resources().fonts().pair(Name(b"F1"), font);
    page.finish();
    pdf.stream(content_id, &content.finish());

    pdf.pages(tree).kids(page_ids.iter().copied()).count(page_ids.len() as i32);
    pdf.catalog(catalog).pages(tree);
    Ok(pdf.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 紙の枚数は_向きを選んで少なくする() {
        let a4 = PrintSettings::default();
        // 刷れる範囲は 190 x 277 mm、重ねしろを引くと 180 x 267 mm ずつ進む。
        let t = tile([150.0, 200.0], &a4);
        assert_eq!((t.columns, t.rows, t.landscape()), (1, 1, false));
        // 700 x 250 mm は、縦向きなら 4 x 1 = 4 枚、横向きなら 3 x 2 = 6 枚。少ない縦向きにする。
        let t = tile([700.0, 250.0], &a4);
        assert_eq!((t.columns, t.rows, t.landscape()), (4, 1, false), "{t:?}");
        // 600 x 180 mm は、横向きなら 3 x 1 = 3 枚（縦向きは 4 x 1）。
        let wide = tile([600.0, 180.0], &a4);
        assert_eq!((wide.columns, wide.rows, wide.landscape()), (3, 1, true), "{wide:?}");
        // A3 なら、もっと少ない。
        let a3 = tile([700.0, 250.0], &PrintSettings { paper: Paper::A3, ..a4 });
        assert!(a3.pages() < t.pages(), "{a3:?}");
        // 重ねしろを含めて、展開図の全体が覆われる。
        for t in [t, a3] {
            assert!(t.step[0] * (t.columns - 1) as f32 + (t.page[0] - 2.0 * a4.margin) >= 700.0);
            assert!(t.step[1] * (t.rows - 1) as f32 + (t.page[1] - 2.0 * a4.margin) >= 250.0);
        }
    }

    #[test]
    fn pdf_は_紙の枚数のページと_1_枚の絵を持つ() {
        let (w, h) = (40, 20);
        let rgba = [200u8, 10, 10, 255].repeat(w * h);
        let cuts = [([0.0, 0.0], [300.0, 0.0])];
        let folds = [([100.0, 0.0], [100.0, 150.0])];
        let sheet = SheetPrint { min: [0.0, 0.0], max: [300.0, 150.0], w, h, rgba: &rgba, cuts: &cuts, folds: &folds, title: "箱 box-1" };
        let settings = PrintSettings::default();
        let bytes = print_pdf(&sheet, &settings).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("%PDF-"));
        let expected = tile([300.0, 150.0], &settings).pages();
        // 絵は全面に紙があるので、すべての紙を刷る。
        assert_eq!(text.matches("/Type /Page\n").count() + text.matches("/Type /Page ").count() + text.matches("/Type /Page/").count(), expected, "{expected} ページのはず");
        assert_eq!(text.matches("/Subtype /Image").count(), 1, "絵は 1 回だけ入れる");
    }

    #[test]
    fn 入稿用の_pdf_は_版を特色で重ねる() {
        let (w, h) = (20, 10);
        let rgba = [0u8, 0, 0, 0].repeat(w * h);
        let mut ink = vec![0u8; w * h];
        ink[..30].fill(255);
        let plates = [crate::art::Plate { name: "Foil Gold".into(), rgb: [212, 175, 55], ink }];
        let cuts = [([0.0, 0.0], [200.0, 0.0])];
        let side = ReadySide { label: "Front", rgba: &rgba, plates: &plates, cuts: &cuts, folds: &[] };
        let bytes = print_ready_pdf([0.0, 0.0], [200.0, 100.0], w, h, &[side], "box", &["- Foil Gold (front)".into()]).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Separation /Foil#20Gold"), "特色の名前");
        assert!(text.contains("/Separation /Die#20Cut") && text.contains("/Separation /Crease"));
        assert!(text.contains("/OP true") && text.contains("/op true"));
        assert!(text.contains("/ImageMask true"));
        assert!(text.contains("/TrimBox"));
        assert_eq!(plate_cmyk([255, 255, 255]), [0.15, 0.0, 0.0, 0.0]);
    }
}
