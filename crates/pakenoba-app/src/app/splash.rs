//! 起動ロゴ（Photoshop の起動画面のようなもの）。
//!
//! 起動してすぐ、ウィンドウの中央にロゴのカードを出し、しばらくしたら薄くして消す。
//! 押せば、すぐ消える。カードの下では、もうアプリが動いている（待たせるためのものではない）。
//! ロゴは、旧パケノバの起動画面と同じ横型（暗い背景用）。名称とロゴは TRADEMARKS.md。

use pakenoba_i18n::t;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Stroke, pos2, vec2};

use super::App;

/// ロゴ（横型、暗い背景用。931 x 309 px）。
const LOGO: &[u8] = include_bytes!("../../assets/pakenoba-logo-horizontal-dark.png");
/// 出しておく時間と、薄くして消す時間（秒）。
const HOLD: f64 = 1.6;
const FADE: f64 = 0.35;
/// カードの色（ロゴの使い方の決まりで、暗い背景用のロゴを置く濃い灰色）と、ブランドのティールと琥珀。
const CARD: Color32 = Color32::from_rgb(0x1e, 0x22, 0x26);
const TEAL: Color32 = Color32::from_rgb(0x08, 0x7e, 0x8b);
const AMBER: Color32 = Color32::from_rgb(0xf2, 0xa9, 0x00);

/// 起動ロゴの状態。
pub struct Splash {
    /// 出しはじめた時刻。まだ出していなければ `None`。
    started: Option<f64>,
    /// 消し終わった、または出さない。
    pub done: bool,
    logo: Option<egui::TextureHandle>,
}

impl Splash {
    pub fn new(show: bool) -> Splash {
        Splash { started: None, done: !show, logo: None }
    }
}

/// PNG を egui の絵にする。
pub fn logo_image() -> Option<egui::ColorImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(LOGO));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    (info.color_type == png::ColorType::Rgba && buf.len() >= w * h * 4).then(|| egui::ColorImage::from_rgba_unmultiplied([w, h], &buf[..w * h * 4]))
}

impl App {
    /// 起動ロゴを、ほかのすべての上に描く。
    pub(super) fn splash(&mut self, ctx: &egui::Context) {
        if self.splash.done {
            return;
        }
        let now = ctx.input(|i| i.time);
        let started = *self.splash.started.get_or_insert(now);
        let age = now - started;
        let fade = ((HOLD + FADE - age) / FADE).clamp(0.0, 1.0) as f32;
        if fade <= 0.0 {
            self.splash.done = true;
            return;
        }
        if self.splash.logo.is_none() {
            self.splash.logo = logo_image().map(|image| ctx.load_texture("splash-logo", image, egui::TextureOptions::LINEAR));
        }
        let screen = ctx.content_rect();
        let card = Rect::from_center_size(screen.center(), vec2(560.0, 330.0).min(screen.size() - vec2(24.0, 24.0)));
        // 押せば、すぐ消える。カードの外を押しても同じ。
        let clicked = egui::Area::new(egui::Id::unique("splash")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let response = ui.allocate_rect(screen, egui::Sense::click());
            let painter = ui.painter();
            let a = |c: Color32| c.gamma_multiply(fade);
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha((90.0 * fade) as u8));
            painter.rect_filled(card.translate(vec2(0.0, 6.0)), 10.0, Color32::from_black_alpha((70.0 * fade) as u8));
            painter.rect(card, 10.0, a(CARD), Stroke::new(1.0, a(Color32::from_gray(60))), egui::StrokeKind::Inside);
            // ブランドの 2 色の帯（シンボルの 4 面の色）。
            let strip = Rect::from_min_max(pos2(card.left(), card.bottom() - 6.0), card.max);
            painter.rect_filled(Rect::from_min_max(strip.min, pos2(strip.center().x, strip.bottom())), egui::CornerRadius { sw: 10, ..Default::default() }, a(TEAL));
            painter.rect_filled(Rect::from_min_max(pos2(strip.center().x, strip.top()), strip.max), egui::CornerRadius { se: 10, ..Default::default() }, a(AMBER));
            if let Some(logo) = &self.splash.logo {
                let size = logo.size_vec2();
                let width = (card.width() - 80.0).min(size.x * 0.5);
                let rect = Rect::from_center_size(pos2(card.center().x, card.top() + 40.0 + width * size.y / size.x * 0.5), vec2(width, width * size.y / size.x));
                painter.image(logo.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), a(Color32::WHITE));
            }
            let text = a(Color32::from_gray(200));
            let weak = a(Color32::from_gray(140));
            let left = card.left() + 32.0;
            painter.text(pos2(left, card.bottom() - 92.0), Align2::LEFT_BOTTOM, t!("Package design, on the box"), FontId::proportional(15.0), text);
            painter.text(pos2(left, card.bottom() - 68.0), Align2::LEFT_BOTTOM, t!("Version {}", crate::VERSION_LABEL), FontId::proportional(12.0), weak);
            painter.text(pos2(left, card.bottom() - 26.0), Align2::LEFT_BOTTOM, t!("The PAKENOBA name and logo are not covered by the code license (MIT or Apache-2.0)."), FontId::proportional(12.0), weak);
            response.clicked()
        });
        if clicked.inner {
            self.splash.done = true;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ロゴが読める() {
        let image = super::logo_image().expect("logo decodes");
        assert_eq!(image.size, [931, 309]);
    }
}
