//! 画像トレース（画像の柄のベクター化）。Illustrator の画像トレースと同じく、欄で値を動かしてプレビューを見てから確定する。
//!
//! - 入り口: レイヤーの右クリック、柄のメニュー、プロパティの「画像トレース…」。
//! - 値を動かすと、少し待ってから別のスレッドでトレースし直し、箱の上の元の画像の所にプレビューを出す。
//!   表示は、トレース結果、元の画像、アウトライン（パスの形だけ）、アウトラインと元の画像を重ねたもの、から選ぶ。
//! - 確定すると、元の画像のレイヤーは隠して残し、その上に、トレースしたベクターのレイヤーを足す（非破壊）。
//!   ベクターのレイヤーには、元の画像のレイヤーとトレースの設定を残すので、「トレースをやり直す…」で同じ欄を開き直せる。
//!   SVG は新しい素材になる。元の素材は変えない。Ctrl+Z で、確定する前に戻せる。

use pakenoba_i18n::t;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;

use pakenoba_core::project::{self, Layer, TraceSource};

use super::assets::{Asset, same_picture};
use super::{App, file};
use crate::paint::{Picture, Placed};
use crate::vector::{ColorKnob, PRESETS, TraceSettings, preset_tune};

/// トレースに渡す絵の、長いほうの画素数の上限。これより大きい画像は、縮めてからトレースする（時間とメモリを抑える）。
const TRACE_LONG: usize = 2500;
/// 値を動かしてから、トレースし直すまでの待ち時間（スライダーを動かしている間に、何度もトレースしないように）。
const SETTLE: Duration = Duration::from_millis(300);

/// プレビューの見せ方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TraceView {
    /// トレース結果。
    Result,
    /// 元の画像。
    Original,
    /// アウトライン（パスの形だけ）。
    Outline,
    /// アウトラインと元の画像を重ねたもの。
    OutlineOverOriginal,
}

/// トレースの結果（プレビューと、確定に使うもの）。
struct Done {
    settings: TraceSettings,
    svg: String,
    paths: usize,
    colors: usize,
    result: Picture,
    outline: Picture,
    overlay: Picture,
}

/// 開いている画像トレースの欄。
pub(super) struct TraceSession {
    /// 元の画像のレイヤーの番号と、開いたときの絵（同じ柄かを確かめる）。
    source: usize,
    picture: Picture,
    /// やり直すときの、トレースしたベクターのレイヤーの番号。
    target: Option<usize>,
    pub(super) settings: TraceSettings,
    pub(super) view: TraceView,
    /// いちばん新しい結果。
    done: Option<Done>,
    /// 動いているトレース（何番目の依頼か、結果）。
    job: Option<(u64, Receiver<Result<Done, String>>)>,
    /// 依頼の番号（古い結果を捨てるため）。
    asked: u64,
    /// 値を変えた時刻（まだトレースに出していない変更があるとき）。
    changed: Option<Instant>,
    error: Option<String>,
    /// 元の画像の名前（新しい素材と、レイヤーの名前に使う）。
    stem: String,
    /// 欄の中のプレビューの絵（元の画像、結果、アウトライン、アウトラインと元の画像）。結果は、どの依頼のものか。
    textures: Option<(u64, [egui::TextureHandle; 3])>,
    original_texture: Option<egui::TextureHandle>,
    /// 欄の中のプレビューの拡大率（1 で、枠に収まる大きさ）と、ずらし（点）。
    zoom: f32,
    pan: egui::Vec2,
}

/// 選んだパスを囲む線の色。
const HIGHLIGHT: [u8; 3] = [0, 170, 255];

/// 欄の中のプレビューの絵の、長いほうの上限（画面に出す絵として十分な細かさ）。
const PREVIEW_LONG: usize = 2048;

/// 絵を、欄のプレビューに出す絵にする。
fn preview_texture(ctx: &egui::Context, name: &str, picture: &Picture) -> Option<egui::TextureHandle> {
    let Picture::Image { w, h, rgba, .. } = picture else { return None };
    let (w, h, rgba) = shrink(*w, *h, rgba, PREVIEW_LONG);
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    Some(ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
}

/// 長いほうが `long` px 以下になるように、画素を平均して縮める。
fn shrink(w: usize, h: usize, rgba: &[u8], long: usize) -> (usize, usize, Vec<u8>) {
    let step = w.max(h).div_ceil(long).max(1);
    if step == 1 {
        return (w, h, rgba.to_vec());
    }
    let (nw, nh) = (w.div_ceil(step), h.div_ceil(step));
    let mut out = Vec::with_capacity(nw * nh * 4);
    for ny in 0..nh {
        for nx in 0..nw {
            // アルファで重みをつけて平均する（透明なところの色が、縁ににじまないように）。
            let (mut sum, mut alpha, mut count) = ([0u32; 3], 0u32, 0u32);
            for y in ny * step..((ny + 1) * step).min(h) {
                for x in nx * step..((nx + 1) * step).min(w) {
                    let p = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    for k in 0..3 {
                        sum[k] += p[k] as u32 * p[3] as u32;
                    }
                    alpha += p[3] as u32;
                    count += 1;
                }
            }
            let a = alpha.max(1);
            out.extend_from_slice(&[(sum[0] / a) as u8, (sum[1] / a) as u8, (sum[2] / a) as u8, (alpha / count.max(1)) as u8]);
        }
    }
    (nw, nh, out)
}

impl App {
    /// 画像トレースの欄を開く。`index` は、画像の柄か、トレースで作ったベクターの柄（やり直す）。
    pub(super) fn open_trace(&mut self, index: usize) {
        if self.trace_session.is_some() {
            return self.say(t!("Image Trace is already open. Apply or cancel it first."));
        }
        let Some(placed) = self.decals.get(index) else { return };
        // トレースで作った柄なら、元の画像の柄から開き直す。
        let (source, target, settings) = match (&placed.picture, &placed.layer.traced) {
            (Picture::Image { vector: false, .. }, _) => (index, None, self.trace),
            (_, Some(traced)) => {
                let Some(source) = self.decals.iter().position(|d| d.layer.id == traced.layer && matches!(d.picture, Picture::Image { vector: false, .. })) else {
                    return self.say(t!("The original image of this trace is no longer in the layers, so it cannot be traced again."));
                };
                (source, Some(index), TraceSettings::from_json(&traced.settings).unwrap_or(self.trace))
            }
            _ => return self.say(t!("Select image artwork (not vector) to trace.")),
        };
        let picture = self.decals[source].picture.clone();
        let base = self.asset_of(&picture).map_or_else(|| self.layer_label(source), |i| self.assets[i].name.clone());
        let stem = base.rsplit_once('.').map_or(base.as_str(), |(stem, _)| stem).to_owned();
        self.trace_session = Some(TraceSession { source, picture, target, settings, view: TraceView::Result, done: None, job: None, asked: 0, changed: Some(Instant::now() - SETTLE), error: None, stem, textures: None, original_texture: None, zoom: 1.0, pan: egui::Vec2::ZERO });
        self.select_only(Some(target.unwrap_or(source)));
    }

    /// 画像トレースの欄の毎回の仕事: 結果を受け取り、値が落ち着いたら、トレースし直す。
    pub(super) fn poll_vectorize(&mut self, ctx: &egui::Context) {
        let Some(session) = &mut self.trace_session else { return };
        // 開いている間に元の画像が消えたり変わったりしたら、閉じる。
        let still = self.decals.get(session.source).is_some_and(|d| same_picture(&d.picture, &session.picture)) && session.target.is_none_or(|t| t < self.decals.len());
        if !still {
            self.trace_session = None;
            (self.art_dirty, self.back_dirty) = (true, true);
            return self.say(t!("The image changed, so Image Trace was closed."));
        }
        if let Some((number, result)) = &session.job {
            match result.try_recv() {
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    session.job = None;
                    session.error = Some(t!("Tracing stopped partway.").to_owned());
                }
                Ok(done) => {
                    let current = *number == session.asked;
                    session.job = None;
                    if current {
                        match done {
                            Ok(done) => {
                                session.done = Some(done);
                                session.error = None;
                            }
                            Err(why) => session.error = Some(why),
                        }
                        (self.art_dirty, self.back_dirty) = (true, true);
                    }
                }
            }
        }
        if session.job.is_none() && session.changed.is_some_and(|at| at.elapsed() >= SETTLE) {
            session.changed = None;
            session.asked += 1;
            let number = session.asked;
            let Picture::Image { w, h, rgba, .. } = &session.picture else { return };
            let (w, h, rgba, size, settings) = (*w, *h, rgba.clone(), self.decals[session.source].decal.size, session.settings);
            let (send, result) = std::sync::mpsc::channel();
            let work = move || {
                let _ = send.send(trace_now(w, h, &rgba, size, settings));
            };
            match std::thread::Builder::new().name("trace".to_owned()).spawn(work) {
                Ok(_) => session.job = Some((number, result)),
                Err(e) => session.error = Some(t!("Could not start vectorizing: {e}")),
            }
        }
        if session.job.is_some() || session.changed.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    /// 描くときに、プレビューを差し込んだ柄の並び。欄を開いていなければ None。
    pub(super) fn trace_preview(&self) -> Option<Vec<Placed>> {
        let session = self.trace_session.as_ref()?;
        let mut decals = self.decals.clone();
        // やり直しのときは、いまのトレースの柄を隠して、元の画像の所に見せる。
        if let Some(target) = session.target {
            decals[target].layer.hidden = true;
        }
        // 箱の上には、トレース結果を出す（アウトラインは、欄の中のプレビューだけで見せる）。「元の画像」なら元の画像。
        let picture = match (session.view, &session.done) {
            (TraceView::Original, _) | (_, None) => session.picture.clone(),
            (_, Some(done)) => done.result.clone(),
        };
        let source = &mut decals[session.source];
        source.picture = picture;
        // 元の画像のレイヤーは、確定したあとは隠してある。欄を開いている間は、調整なしで見せる。
        source.layer = Layer { id: source.layer.id, depth: source.layer.depth, ..Layer::default() };
        Some(decals)
    }

    /// 選んでいるパスを、水色の線で囲んだ柄の並び（パスを選んでいなければ None）。
    pub(super) fn path_highlight(&self, decals: &[Placed]) -> Option<Vec<Placed>> {
        let (index, path, _) = self.path_pick?;
        let placed = decals.get(index).filter(|p| matches!(p.picture, Picture::Image { vector: true, .. }))?;
        let mut style = placed.layer.vector.clone().unwrap_or_default();
        let mark = Some((HIGHLIGHT, 60));
        match style.paths.iter_mut().find(|p| p.index == path) {
            Some(p) => p.stroke = mark,
            None => style.paths.push(pakenoba_core::project::PathStyle { stroke: mark, ..pakenoba_core::project::PathStyle::new(path) }),
        }
        let mut out = decals.to_vec();
        out[index].layer.vector = Some(style);
        Some(out)
    }

    /// ベクターの柄 `index` の、柄の座標 `grab`（mm、中心が原点）にあるパスを選ぶ。選べたら真。
    pub(super) fn pick_path(&mut self, index: usize, grab: [f32; 2]) -> bool {
        let Some(placed) = self.decals.get(index) else { return false };
        let Picture::Image { rgba, vector: true, .. } = &placed.picture else { return false };
        let Some(svg) = crate::vector::svg_of(rgba) else {
            self.say(t!("The shapes of this vector artwork cannot be read."));
            return false;
        };
        // 隠したパスは選ばない（見えているものから選ぶ）。線だけ表示でも、形の中を押せば選べるように、塗りで見分ける。
        let text = match placed.layer.vector.clone().filter(|v| v.active()) {
            Some(mut style) => {
                style.outline_only = false;
                crate::vector::restyle(&svg, &style)
            }
            None => svg.to_string(),
        };
        let size = placed.decal.size;
        // 裏の柄は、左右を逆にして描いている。
        let x = grab[0] / size[0] * if placed.back { -1.0 } else { 1.0 } + 0.5;
        let y = 0.5 - grab[1] / size[1];
        match crate::vector::path_at(&text, [x, y]) {
            Some(path) => {
                let color = crate::vector::shape_colors(&svg).get(path as usize).copied().flatten();
                self.select_only(Some(index));
                self.path_pick = Some((index, path, color));
                (self.adjust_focus, self.effect_focus) = (None, Some((index, super::layers::Effect::Vector)));
                (self.art_dirty, self.back_dirty) = (true, true);
                self.say(t!("Selected path {} of this vector artwork. Change it in Properties; press Esc to deselect.", path + 1));
                true
            }
            None => {
                self.say(t!("There is no path there."));
                false
            }
        }
    }

    /// 確定: 元の画像のレイヤーを隠して残し、その上にベクターのレイヤーを足す（やり直しなら、そのレイヤーの絵を替える）。
    fn apply_trace(&mut self) {
        let Some(session) = self.trace_session.take() else { return };
        (self.art_dirty, self.back_dirty) = (true, true);
        let Some(done) = session.done else { return };
        self.ensure_ids();
        let source = session.source;
        let size = self.decals[source].decal.size;
        let name = t!("{} (vector).svg", session.stem);
        if let Picture::Image { rgba, .. } = &done.result {
            crate::vector::remember_svg(rgba, done.svg.as_bytes());
        }
        let stored = project::Asset::new(name.clone(), done.svg.into_bytes(), size, file::now_local());
        self.checkpoint();
        let known = self.assets.iter().find(|a| a.stored.as_ref().is_some_and(|s| s.id() == stored.id())).map(|a| a.picture.clone());
        let picture = known.unwrap_or_else(|| {
            let picture = done.result.clone();
            self.assets.push(Asset { name: name.clone(), picture: picture.clone(), size: stored.size, thumb: None, bundled: false, stored: Some(stored) });
            picture
        });
        let traced = TraceSource { layer: self.decals[source].layer.id, settings: done.settings.to_json() };
        self.trace = done.settings;
        let index = match session.target {
            Some(target) => {
                self.decals[target].picture = picture;
                self.decals[target].layer.traced = Some(traced);
                target
            }
            None => {
                let original = &self.decals[source];
                let layer = Layer { name: Some(t!("{} (trace)", self.layer_label(source))), depth: original.layer.depth, traced: Some(traced), ..Layer::default() };
                let placed = Placed { decal: original.decal, picture, back: original.back, layer };
                self.decals.insert(source + 1, placed);
                self.decals[source].layer.hidden = true;
                self.tidy_groups();
                self.ensure_ids();
                source + 1
            }
        };
        self.select_only(Some(index));
        self.say(t!("Traced ({} shapes, {} colors). The original image is kept, hidden, below it. Press Ctrl+Z to undo.", done.paths, done.colors));
    }

    /// 閉じる（何も変えない）。
    fn cancel_trace(&mut self) {
        if self.trace_session.take().is_some() {
            (self.art_dirty, self.back_dirty) = (true, true);
        }
    }

    /// 画像トレースの欄。
    pub(super) fn trace_window(&mut self, ctx: &egui::Context) {
        let Some(session) = &mut self.trace_session else { return };
        // 欄の中のプレビューの絵を用意する（結果が新しくなったら作り直す）。
        if session.original_texture.is_none() {
            session.original_texture = preview_texture(ctx, "trace-original", &session.picture);
        }
        if let Some(done) = &session.done
            && session.textures.as_ref().is_none_or(|(n, _)| *n != session.asked)
            && let (Some(a), Some(b), Some(c)) = (preview_texture(ctx, "trace-result", &done.result), preview_texture(ctx, "trace-outline", &done.outline), preview_texture(ctx, "trace-overlay", &done.overlay))
        {
            session.textures = Some((session.asked, [a, b, c]));
        }
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        let mut changed = false;
        let view_before = session.view;
        // はじめは、立体の表示の右上（プロパティの欄の左）に出す。動かせる。
        let right = ctx.content_rect().right();
        egui::Window::new(t!("Image Trace"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([460.0, 760.0])
            .default_pos(egui::pos2((right - 820.0).max(60.0), 100.0))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.weak(t!("Source: {}", session.stem));
                ui.horizontal_wrapped(|ui| {
                    for (view, label) in [(TraceView::Result, t!("Result")), (TraceView::Outline, t!("Outlines")), (TraceView::OutlineOverOriginal, t!("Outlines with source")), (TraceView::Original, t!("Original"))] {
                        ui.selectable_value(&mut session.view, view, label);
                    }
                });
                // プレビュー。ホイールで拡大縮小、ドラッグで動かす、ダブルクリックで枠に収める。
                let texture = match (session.view, &session.textures) {
                    (TraceView::Original, _) | (_, None) => session.original_texture.as_ref(),
                    (TraceView::Result, Some((_, t))) => Some(&t[0]),
                    (TraceView::Outline, Some((_, t))) => Some(&t[1]),
                    (TraceView::OutlineOverOriginal, Some((_, t))) => Some(&t[2]),
                };
                let side = ui.available_width().max(200.0);
                let (rect, response) = ui.allocate_exact_size(egui::vec2(side, (side * 0.8).clamp(220.0, 520.0)), egui::Sense::click_and_drag());
                let painter = ui.painter().with_clip_rect(rect);
                // 透明な所が分かるように、市松模様の上に置く（アウトラインは白地の方が見やすいので、明るい市松）。
                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(236));
                let cell = 10.0;
                for row in 0..(rect.height() / cell).ceil() as i32 {
                    for col in 0..(rect.width() / cell).ceil() as i32 {
                        if (row + col) % 2 == 0 {
                            let min = rect.min + egui::vec2(col as f32 * cell, row as f32 * cell);
                            painter.rect_filled(egui::Rect::from_min_size(min, egui::vec2(cell, cell)), 0.0, egui::Color32::from_gray(214));
                        }
                    }
                }
                if response.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        let before = session.zoom;
                        session.zoom = (session.zoom * (scroll * 0.0025).exp()).clamp(0.5, 8.0);
                        // カーソルの所を中心に拡大する。
                        if let Some(p) = response.hover_pos() {
                            let from_center = p - rect.center() - session.pan;
                            session.pan -= from_center * (session.zoom / before - 1.0);
                        }
                    }
                }
                if response.dragged() {
                    session.pan += response.drag_delta();
                }
                if response.double_clicked() {
                    (session.zoom, session.pan) = (1.0, egui::Vec2::ZERO);
                }
                if let Some(texture) = texture {
                    let size = texture.size_vec2();
                    let fit = (rect.width() / size.x).min(rect.height() / size.y) * 0.94;
                    let shown = egui::Rect::from_center_size(rect.center() + session.pan, size * fit * session.zoom);
                    painter.image(texture.id(), shown, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
                if session.job.is_some() || session.changed.is_some() {
                    // トレースし直している間は、右上に小さく出す。
                    let at = rect.right_top() + egui::vec2(-22.0, 22.0);
                    painter.circle_filled(at, 13.0, egui::Color32::from_black_alpha(140));
                    // ui.put で置くと、後に続く部品の位置がプレビューの上へずれるので、描くだけにする。
                    egui::Spinner::new().size(14.0).color(egui::Color32::WHITE).paint_at(ui, egui::Rect::from_center_size(at, egui::vec2(16.0, 16.0)));
                    ui.ctx().request_repaint();
                }
                painter.text(rect.left_bottom() + egui::vec2(6.0, -6.0), egui::Align2::LEFT_BOTTOM, format!("{:.0}%", session.zoom * 100.0), egui::FontId::proportional(11.0), egui::Color32::from_gray(90));
                response.on_hover_text(t!("Scroll to zoom, drag to move, double-click to fit"));
                ui.separator();
                let settings = &mut session.settings;
                egui::Grid::new("trace-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label(t!("Preset"));
                    let current = PRESETS.get(settings.preset).map_or("", |p| pakenoba_i18n::tr(p.1));
                    egui::ComboBox::from_id_salt("trace-preset").width(170.0).selected_text(current).show_ui(ui, |ui| {
                        for (i, (_, label)) in PRESETS.iter().enumerate() {
                            if ui.selectable_value(&mut settings.preset, i, pakenoba_i18n::tr(label)).changed() {
                                // プリセットを選び直したら、細かい値はプリセットのものに戻す。
                                settings.tune = None;
                                changed = true;
                            }
                        }
                    });
                    ui.end_row();
                    let (_, knob) = preset_tune(settings.preset);
                    let mut tune = settings.tune_now();
                    let before = tune;
                    match knob {
                        ColorKnob::Threshold => {
                            ui.label(t!("Black threshold"));
                            ui.add(egui::Slider::new(&mut tune.threshold, 0..=255)).on_hover_text(t!("Pixels darker than this become black"));
                            ui.end_row();
                        }
                        ColorKnob::Colors => {
                            ui.label(t!("Colors"));
                            ui.add(egui::Slider::new(&mut tune.colors, 2..=30));
                            ui.end_row();
                        }
                        ColorKnob::Detail => {
                            ui.label(t!("Color detail"));
                            ui.add(egui::Slider::new(&mut tune.color_detail, 0.0..=100.0).max_decimals(0)).on_hover_text(t!("Fewer to more colors"));
                            ui.end_row();
                        }
                        ColorKnob::None => {}
                    }
                    ui.label(t!("Paths"));
                    ui.add(egui::Slider::new(&mut tune.paths, 0.0..=100.0).max_decimals(0).suffix(" %")).on_hover_text(t!("Higher follows the picture more closely, with more points"));
                    ui.end_row();
                    ui.label(t!("Corners"));
                    ui.add(egui::Slider::new(&mut tune.corners, 0.0..=100.0).max_decimals(0).suffix(" %")).on_hover_text(t!("Higher keeps more sharp corners"));
                    ui.end_row();
                    ui.label(t!("Noise"));
                    ui.add(egui::Slider::new(&mut tune.noise, 1..=200).suffix(" px").logarithmic(true)).on_hover_text(t!("Areas smaller than this are left out"));
                    ui.end_row();
                    ui.label(t!("Create"));
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut tune.fills, t!("Fills"));
                        ui.checkbox(&mut tune.strokes, t!("Strokes")).on_hover_text(t!("Thin lines become strokes (center lines)"));
                    });
                    ui.end_row();
                    if tune != before {
                        settings.tune = Some(tune);
                        changed = true;
                    }
                    ui.label("");
                    changed |= ui.checkbox(&mut settings.drop_background, t!("Leave out the background")).on_hover_text(t!("Does not make shapes from the area connected to the edges in the most common edge color")).changed();
                    ui.end_row();
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if session.job.is_some() || session.changed.is_some() {
                        ui.spinner();
                        ui.weak(t!("Tracing…"));
                    } else if let Some(why) = &session.error {
                        ui.colored_label(ui.visuals().warn_fg_color, why);
                    } else if let Some(done) = &session.done {
                        ui.weak(t!("{} shapes, {} colors", done.paths, done.colors));
                    }
                });
                ui.horizontal(|ui| {
                    let ready = session.done.is_some() && session.job.is_none() && session.changed.is_none();
                    let label = if session.target.is_some() { t!("Apply") } else { t!("Trace") };
                    if ui.add_enabled(ready, egui::Button::new(egui::RichText::new(label).strong())).on_hover_text(t!("Adds the vector layer above the image. The image is kept, hidden")).clicked() {
                        apply = true;
                    }
                    if ui.button(t!("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if changed {
            session.changed = Some(Instant::now());
            ctx.request_repaint();
        }
        if session.view != view_before {
            (self.art_dirty, self.back_dirty) = (true, true);
        }
        if apply {
            self.apply_trace();
        } else if cancel || !open {
            self.cancel_trace();
        }
    }
}

/// トレースして、プレビューの絵（結果、アウトライン、アウトラインと元の画像）まで作る。
fn trace_now(w: usize, h: usize, rgba: &[u8], size: [f32; 2], settings: TraceSettings) -> Result<Done, String> {
    let (tw, th, small) = shrink(w, h, rgba, TRACE_LONG);
    let traced = crate::vector::trace(tw, th, &small, settings, size)?;
    let (rw, rh, result) = crate::vector::decode_svg(traced.svg.as_bytes())?;
    let (ow, oh, outline) = crate::vector::decode_svg(traced.outline.as_bytes())?;
    // 元の画像を薄くした上に、アウトラインを重ねる。
    let mut overlay = outline.clone();
    for y in 0..oh {
        for x in 0..ow {
            let (sx, sy) = ((x * w / ow).min(w - 1), (y * h / oh).min(h - 1));
            let s = &rgba[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4];
            let o = (y * ow + x) * 4;
            let line = overlay[o + 3] as f32 / 255.0;
            let a = s[3] as f32 / 255.0 * 0.55;
            for k in 0..3 {
                let faded = s[k] as f32 * 0.5 + 255.0 * 0.5;
                let under = faded * a / (a + (1.0 - a) * 0.0).max(1e-3);
                overlay[o + k] = (outline[o + k] as f32 * line + under * (1.0 - line)).round() as u8;
            }
            overlay[o + 3] = ((line + a * (1.0 - line)) * 255.0).round() as u8;
        }
    }
    let image = |w, h, rgba: Vec<u8>, vector| Picture::Image { w, h, rgba: Arc::new(rgba), vector };
    Ok(Done { settings, svg: traced.svg, paths: traced.paths, colors: traced.colors, result: image(rw, rh, result, true), outline: image(ow, oh, outline, false), overlay: image(ow, oh, overlay, false) })
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;
    use crate::paint::Placed;
    use pakenoba_core::surface::Decal;

    /// 白地に、赤い丸。
    fn red_dot(w: usize, h: usize) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                if (x - w as f32 / 2.0).hypot(y - h as f32 / 2.0) < h as f32 / 3.0 { [210, 30, 40, 255] } else { [250, 250, 250, 255] }
            })
            .collect()
    }

    /// プレビューができるまで待つ。
    fn wait(app: &mut App) {
        let ctx = egui::Context::default();
        for _ in 0..600 {
            app.poll_vectorize(&ctx);
            let session = app.trace_session.as_ref().unwrap();
            if session.done.is_some() && session.job.is_none() && session.changed.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("トレースが終わらない");
    }

    fn with_dot() -> (App, usize, Picture) {
        let mut app = App::new(&Initial::default());
        let panel = app.template.panels.iter().position(|p| p.name == "正面").unwrap();
        let picture = Picture::Image { w: 80, h: 60, rgba: Arc::new(red_dot(80, 60)), vector: false };
        let decal = Decal { panel, at: [0.5, 0.5], size: [40.0, 30.0], angle: 0.0 };
        app.decals.push(Placed { decal, picture: picture.clone(), back: false, layer: Default::default() });
        let index = app.decals.len() - 1;
        (app, index, picture)
    }

    #[test]
    fn トレースは元の画像を残して_上にベクターのレイヤーを足す() {
        let (mut app, index, picture) = with_dot();
        let (count, assets) = (app.decals.len(), app.assets.len());
        app.open_trace(index);
        wait(&mut app);
        // プレビューの間は、柄の並びは変わらない。描くときだけ、結果を差し込む。
        assert_eq!(app.decals.len(), count);
        let shown = app.trace_preview().unwrap();
        assert!(matches!(shown[index].picture, Picture::Image { vector: true, .. }));
        assert!(same_picture(&app.decals[index].picture, &picture));
        // 値を変えると、トレースし直す。
        app.trace_session.as_mut().unwrap().settings.preset = 2;
        app.trace_session.as_mut().unwrap().changed = Some(Instant::now() - SETTLE);
        wait(&mut app);
        app.apply_trace();
        assert!(app.trace_session.is_none());
        assert_eq!(app.decals.len(), count + 1);
        assert!(app.decals[index].layer.hidden, "元の画像は隠して残す");
        assert!(same_picture(&app.decals[index].picture, &picture));
        let traced = &app.decals[index + 1];
        assert!(matches!(traced.picture, Picture::Image { vector: true, .. }));
        assert_eq!(traced.decal.size, [40.0, 30.0], "大きさは変わらない");
        let source = traced.layer.traced.as_ref().expect("元とトレースの設定を残す");
        assert_eq!(source.layer, app.decals[index].layer.id);
        assert_eq!(TraceSettings::from_json(&source.settings).unwrap().preset, 2);
        assert_eq!(app.assets.len(), assets + 1);
        assert!(crate::vector::is_svg(app.assets.last().unwrap().stored.as_ref().unwrap().bytes()));
        // （保存して開き直しても残ることは、中核の案件の読み書きのテストで見ている。）
        // やり直す: 元の画像から開き直し、確定すると、同じレイヤーの絵を替える。
        app.open_trace(index + 1);
        assert_eq!(app.trace_session.as_ref().unwrap().source, index);
        assert_eq!(app.trace_session.as_ref().unwrap().settings.preset, 2);
        wait(&mut app);
        app.apply_trace();
        assert_eq!(app.decals.len(), count + 1);
        // 元に戻すと、トレースする前。
        app.undo();
        app.undo();
        assert_eq!(app.decals.len(), count);
        assert!(!app.decals[index].layer.hidden);
    }

    #[test]
    fn 取り消すと_何も変わらない() {
        let (mut app, index, picture) = with_dot();
        let (count, assets) = (app.decals.len(), app.assets.len());
        app.open_trace(index);
        wait(&mut app);
        app.cancel_trace();
        assert_eq!(app.decals.len(), count);
        assert_eq!(app.assets.len(), assets);
        assert!(same_picture(&app.decals[index].picture, &picture));
        assert!(app.trace_preview().is_none());
    }

    #[test]
    fn 途中で柄を消すと_欄を閉じる() {
        let (mut app, index, _) = with_dot();
        app.open_trace(index);
        app.decals.remove(index);
        app.poll_vectorize(&egui::Context::default());
        assert!(app.trace_session.is_none());
    }

    #[test]
    fn 設定は_json_で残して読める() {
        let settings = TraceSettings { preset: 3, drop_background: false, tune: Some(crate::vector::Tune { paths: 77.0, ..Default::default() }) };
        assert_eq!(TraceSettings::from_json(&settings.to_json()), Some(settings));
    }

    #[test]
    fn 大きい画像は縮めてからトレースする() {
        let (w, h, rgba) = shrink(5000, 100, &vec![255u8; 5000 * 100 * 4], TRACE_LONG);
        assert_eq!((w, h), (2500, 50));
        assert_eq!(rgba.len(), w * h * 4);
        assert_eq!(shrink(100, 50, &vec![0u8; 100 * 50 * 4], TRACE_LONG).0, 100);
    }
}
