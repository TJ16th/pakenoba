//! 手元の AI（Ollama）で、素材に見て分かる名前とタグを付ける。
//!
//! - 絵は、この PC（か、使う人が決めた所）で動く Ollama にだけ送る。外のサービスには送らない。
//! - 画像を見られるモデル（gemma3、qwen2.5vl、llama3.2-vision など）に、縮めた絵（長いほう 448 画素）を渡し、
//!   JSON の形（名前とタグ）を決めて答えさせる（Ollama の `format`）。
//! - 1 枚ずつ、別のスレッドで頼む。画面は止めない。
//! - 案件の素材は、素材（`project::Asset`）の `label` と `tags` に入れる（案件のファイルに残る）。
//!   登録したフォルダの画像は、設定のフォルダの `asset_tags.json` に、ファイルの場所と更新日時で覚える。

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use pakenoba_i18n::t;
use serde::{Deserialize, Serialize};

use super::App;
use crate::paint::Picture;

/// 手元の AI の設定（設定のファイルの `local_ai`）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct LocalAi {
    /// Ollama のアドレス。
    #[serde(default = "default_url")]
    pub url: String,
    /// 使うモデル（画像を見られるもの）。空なら、まだ選んでいない。
    #[serde(default)]
    pub model: String,
    /// 素材を取り込んだら、すぐに名前とタグを付ける。
    #[serde(default)]
    pub auto: bool,
}

fn default_url() -> String {
    "http://localhost:11434".to_owned()
}

impl Default for LocalAi {
    fn default() -> LocalAi {
        LocalAi { url: default_url(), model: String::new(), auto: false }
    }
}

impl LocalAi {
    pub fn ready(&self) -> bool {
        !self.model.trim().is_empty() && !self.url.trim().is_empty()
    }
}

/// 名前とタグ。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Tagged {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 何に付けるか。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Target {
    /// 案件の素材（中身の印）。
    Asset(String),
    /// 登録したフォルダの画像。
    File(PathBuf),
}

struct Job {
    target: Target,
    png: Vec<u8>,
    url: String,
    model: String,
}

/// 手元の AI とのやりとり（App が 1 つ持つ）。
#[derive(Default)]
pub(crate) struct Tagger {
    jobs: Option<Sender<Job>>,
    results: Option<Receiver<(Target, Result<Tagged, String>)>>,
    /// 頼んだが、まだ答えのないもの。
    pub waiting: Vec<Target>,
    /// まとめて頼んだ数と、終わった数（進み具合の表示）。
    pub batch: (usize, usize),
    /// 最後の失敗。
    pub error: Option<String>,
    /// モデルの一覧を調べた結果（画像を見られるものだけ）。
    pub models: Option<Result<Vec<String>, String>>,
    models_rx: Option<Receiver<Result<Vec<String>, String>>>,
    /// 設定の窓を開いているか。
    pub open: bool,
    /// フォルダの画像の名前とタグ（場所 → 更新日時と、名前とタグ）。
    pub index: Option<BTreeMap<String, (u64, Tagged)>>,
}

/// 頼む言葉。パッケージの素材を一覧から探すための名前とタグ（日本語）。
const PROMPT: &str = "この画像は、パッケージ（箱）のデザインに使う素材です。透明な所は灰色で表しています。\
素材の一覧から探しやすくするために、次の 2 つを日本語で答えてください。\n\
- name: 見て分かる短い名前（15 文字以内の名詞句。例: 金色の龍、等高線の模様、カメラを持つ男性、桜の枝のイラスト）\n\
- tags: 探すときに使う言葉を 3〜8 個（写っている物、色、画風、種類。例: 龍、金、和柄、線画、写真、ロゴ、文字、背景、模様、透明背景）\n\
画像に文字があれば、その文字も tags に入れてください。";

/// 答えの形（JSON Schema）。
fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["name", "tags"]
    })
}

/// 答えを、名前とタグにする（長すぎるもの、空のもの、重なりを除く）。
pub(crate) fn parse_answer(text: &str) -> Result<Tagged, String> {
    #[derive(Deserialize)]
    struct Answer {
        #[serde(default)]
        name: String,
        #[serde(default)]
        tags: Vec<String>,
    }
    // モデルによっては、JSON の前後に文字や ``` を付けるので、{ から } までを取り出す。
    let body = match (text.find('{'), text.rfind('}')) {
        (Some(a), Some(b)) if b > a => &text[a..=b],
        _ => text,
    };
    let answer: Answer = serde_json::from_str(body).map_err(|e| t!("The local AI did not answer in the expected form: {e}"))?;
    let clean = |s: &str, n: usize| -> String { s.trim().trim_matches(|c: char| c == '"' || c == '「' || c == '」' || c == '#').chars().take(n).collect::<String>().trim().to_owned() };
    let label = Some(clean(&answer.name, 30)).filter(|s| !s.is_empty());
    let mut tags: Vec<String> = Vec::new();
    for tag in &answer.tags {
        let tag = clean(tag, 20);
        if !tag.is_empty() && !tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) && tags.len() < 12 {
            tags.push(tag);
        }
    }
    if label.is_none() && tags.is_empty() {
        return Err(t!("The local AI gave no name or tags.").to_owned());
    }
    Ok(Tagged { label, tags })
}

/// 絵を、長いほう `long` 画素に縮め、透明な所を灰色にした PNG にする。
pub(crate) fn png_for_ai(w: usize, h: usize, rgba: &[u8], long: usize) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let scale = (long as f32 / w.max(h) as f32).min(1.0);
    let (ow, oh) = (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1));
    let mut out = vec![0u8; ow * oh * 3];
    for y in 0..oh {
        for x in 0..ow {
            // 近い画素の平均（縮めるとき、細い線が消えないように、範囲の画素をならす）。
            let (x0, x1) = (x * w / ow, ((x + 1) * w / ow).max(x * w / ow + 1).min(w));
            let (y0, y1) = (y * h / oh, ((y + 1) * h / oh).max(y * h / oh + 1).min(h));
            let mut sum = [0f32; 3];
            let mut n = 0f32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &rgba[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4];
                    let a = p[3] as f32 / 255.0;
                    for k in 0..3 {
                        sum[k] += p[k] as f32 * a + 160.0 * (1.0 - a);
                    }
                    n += 1.0;
                }
            }
            for k in 0..3 {
                out[(y * ow + x) * 3 + k] = (sum[k] / n).round() as u8;
            }
        }
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, ow as u32, oh as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&out).ok()?;
    }
    Some(png)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn agent(seconds: u64) -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(std::time::Duration::from_secs(seconds))).build().into()
}

fn base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_owned()
}

/// 1 枚ぶん、Ollama に頼む。
fn ask(job: &Job) -> Result<Tagged, String> {
    let body = serde_json::json!({
        "model": job.model,
        "stream": false,
        "keep_alive": "10m",
        "format": schema(),
        "options": { "temperature": 0.2 },
        "messages": [{ "role": "user", "content": PROMPT, "images": [base64(&job.png)] }]
    });
    let mut response = agent(300).post(format!("{}/api/chat", base(&job.url))).header("content-type", "application/json").send(body.to_string()).map_err(|e| t!("Could not reach Ollama at {}: {e}", job.url))?;
    let status = response.status();
    let text = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if !status.is_success() {
        let why = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["error"].as_str().map(str::to_owned)).unwrap_or(text);
        return Err(t!("Ollama answered with an error: {why}"));
    }
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    parse_answer(value["message"]["content"].as_str().unwrap_or_default())
}

/// 入っているモデルのうち、画像を見られるもの。
fn vision_models(url: &str) -> Result<Vec<String>, String> {
    let agent = agent(20);
    let mut response = agent.get(format!("{}/api/tags", base(url))).call().map_err(|e| t!("Could not reach Ollama at {url}: {e}"))?;
    let list: serde_json::Value = serde_json::from_str(&response.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for model in list["models"].as_array().into_iter().flatten() {
        let Some(name) = model["name"].as_str() else { continue };
        let shown: Option<serde_json::Value> = agent
            .post(format!("{}/api/show", base(url)))
            .header("content-type", "application/json")
            .send(serde_json::json!({ "model": name }).to_string())
            .ok()
            .and_then(|mut r| r.body_mut().read_to_string().ok())
            .and_then(|text| serde_json::from_str(&text).ok());
        let vision = shown.as_ref().and_then(|s| s["capabilities"].as_array()).is_some_and(|c| c.iter().any(|c| c == "vision"));
        if vision {
            out.push(name.to_owned());
        }
    }
    Ok(out)
}

/// フォルダの画像の、覚えておく鍵（場所）と、更新日時（秒）。
fn file_key(path: &Path) -> (String, u64) {
    let modified = std::fs::metadata(path).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    (path.to_string_lossy().into_owned(), modified)
}

impl App {
    /// 設定のフォルダの `asset_tags.json`。
    fn tag_index_path(&self) -> Option<PathBuf> {
        Some(self.settings.folder()?.join("asset_tags.json"))
    }

    fn tag_index(&mut self) -> &mut BTreeMap<String, (u64, Tagged)> {
        if self.tagger.index.is_none() {
            let read = self.tag_index_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            self.tagger.index = Some(read);
        }
        self.tagger.index.get_or_insert_with(Default::default)
    }

    fn save_tag_index(&mut self) {
        let (Some(path), Some(index)) = (self.tag_index_path(), self.tagger.index.as_ref()) else { return };
        if let Ok(text) = serde_json::to_vec(index) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::File::create(&tmp).and_then(|mut f| f.write_all(&text)).and_then(|()| std::fs::rename(&tmp, &path)).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    /// 登録したフォルダの画像の、名前とタグ（ファイルが書き換わっていれば、ないものとする）。
    pub(super) fn file_tags(&self, path: &Path) -> Option<Tagged> {
        let (key, modified) = file_key(path);
        self.tagger.index.as_ref()?.get(&key).filter(|(m, _)| *m == modified).map(|(_, t)| t.clone())
    }

    /// フォルダの画像の名前とタグの一覧を、まだなら読む。
    pub(super) fn load_tag_index(&mut self) {
        let _ = self.tag_index();
    }

    /// 登録したフォルダの画像の、名前とタグを書き換える。
    pub(super) fn set_file_tags(&mut self, path: &Path, tagged: Tagged) {
        let (key, modified) = file_key(path);
        self.tag_index().insert(key, (modified, tagged));
        self.save_tag_index();
    }

    /// 頼む。絵は、案件の素材ならその絵、フォルダの画像なら読み込んで縮めたもの。
    pub(super) fn request_tags(&mut self, target: Target) {
        let ai = self.settings.local_ai.clone();
        if !ai.ready() || self.tagger.waiting.contains(&target) {
            return;
        }
        let png = match &target {
            Target::Asset(id) => self.assets.iter().find(|a| a.stored.as_ref().is_some_and(|s| s.id() == id)).and_then(|a| match &a.picture {
                Picture::Image { w, h, rgba, .. } => png_for_ai(*w, *h, rgba, 448),
                _ => None,
            }),
            Target::File(path) => super::library::small_rgba(path, 448).and_then(|(w, h, rgba)| png_for_ai(w, h, &rgba, 448)),
        };
        let Some(png) = png else {
            self.tagger.error = Some(t!("This picture cannot be sent to the local AI.").to_owned());
            return;
        };
        if self.tagger.jobs.is_none() {
            let (jobs_tx, jobs_rx) = channel::<Job>();
            let (results_tx, results_rx) = channel();
            std::thread::spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    let result = ask(&job);
                    if results_tx.send((job.target, result)).is_err() {
                        break;
                    }
                }
            });
            self.tagger.jobs = Some(jobs_tx);
            self.tagger.results = Some(results_rx);
        }
        if let Some(jobs) = &self.tagger.jobs
            && jobs.send(Job { target: target.clone(), png, url: ai.url, model: ai.model }).is_ok()
        {
            self.tagger.waiting.push(target);
        }
    }

    /// 答えを受け取って、素材と一覧に入れる。毎フレーム呼ぶ。
    pub(super) fn poll_tags(&mut self, ctx: &eframe::egui::Context) {
        if let Some(rx) = &self.tagger.models_rx
            && let Ok(found) = rx.try_recv()
        {
            if let Ok(list) = &found
                && self.settings.local_ai.model.is_empty()
                && let Some(first) = list.first()
            {
                self.settings.local_ai.model = first.clone();
                let _ = self.settings.save();
            }
            self.tagger.models = Some(found);
            self.tagger.models_rx = None;
        }
        let mut got = Vec::new();
        if let Some(rx) = &self.tagger.results {
            while let Ok(answer) = rx.try_recv() {
                got.push(answer);
            }
        }
        if !self.tagger.waiting.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
        for (target, result) in got {
            self.tagger.waiting.retain(|t| *t != target);
            self.tagger.batch.1 += 1;
            match result {
                Ok(tagged) => {
                    self.tagger.error = None;
                    self.apply_tags(&target, tagged);
                }
                Err(e) => {
                    // 1 回失敗したら、残りを頼むのはやめる（Ollama が止まっているなど）。
                    self.tagger.error = Some(e);
                    self.cancel_tags();
                }
            }
        }
        if self.tagger.waiting.is_empty() {
            self.tagger.batch = (0, 0);
        }
    }

    /// 答えを入れる。手で付けた名前（AI のものでない名前）は、上書きしない。
    fn apply_tags(&mut self, target: &Target, tagged: Tagged) {
        match target {
            Target::Asset(id) => {
                if let Some(stored) = self.assets.iter_mut().filter_map(|a| a.stored.as_mut()).find(|s| s.id() == id) {
                    if stored.label.is_none() {
                        stored.label = tagged.label;
                    }
                    for tag in tagged.tags {
                        if !stored.tags.contains(&tag) {
                            stored.tags.push(tag);
                        }
                    }
                }
            }
            Target::File(path) => {
                self.load_tag_index();
                let mut now = self.file_tags(path).unwrap_or_default();
                if now.label.is_none() {
                    now.label = tagged.label;
                }
                for tag in tagged.tags {
                    if !now.tags.contains(&tag) {
                        now.tags.push(tag);
                    }
                }
                self.set_file_tags(path, now);
            }
        }
    }

    /// まだ答えのないものを、頼まなかったことにする（スレッドは、いま頼んでいる 1 枚を終えてから止まる）。
    pub(super) fn cancel_tags(&mut self) {
        self.tagger.jobs = None;
        self.tagger.results = None;
        self.tagger.waiting.clear();
        self.tagger.batch = (0, 0);
    }

    /// まとめて頼む（まだ名前もタグもないものだけ）。
    pub(super) fn request_many(&mut self, targets: Vec<Target>) {
        self.track("ai_used", serde_json::json!({ "feature": "local_ai_tags", "count": targets.len() }));
        let before = self.tagger.waiting.len();
        for target in targets {
            self.request_tags(target);
        }
        let added = self.tagger.waiting.len() - before;
        self.tagger.batch.0 += added;
    }

    /// 環境設定の「手元の AI（Ollama）…」の窓。
    pub(super) fn local_ai_window(&mut self, ctx: &eframe::egui::Context) {
        use eframe::egui;
        if !self.tagger.open {
            return;
        }
        let mut open = true;
        let mut changed = false;
        egui::Window::new(t!("Local AI (Ollama)")).open(&mut open).resizable(false).collapsible(false).default_width(420.0).show(ctx, |ui| {
            ui.label(t!("Gives assets a name you can recognize and tags to search by, using a vision model running in Ollama on this computer. Pictures are sent only to this Ollama, never to an outside service."));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(t!("Address"));
                changed |= ui.add(egui::TextEdit::singleline(&mut self.settings.local_ai.url).desired_width(220.0)).lost_focus();
                if ui.button(t!("Connect")).on_hover_text(t!("Lists the installed models that can look at images")).clicked() {
                    let url = self.settings.local_ai.url.clone();
                    let (tx, rx) = channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(vision_models(&url));
                    });
                    self.tagger.models_rx = Some(rx);
                    self.tagger.models = None;
                }
            });
            if self.tagger.models_rx.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t!("Asking Ollama…"));
                });
            }
            match &self.tagger.models {
                Some(Ok(list)) if list.is_empty() => {
                    ui.colored_label(super::theme::WARN, t!("No model that can look at images is installed. Install one, for example: ollama pull gemma3 (or qwen2.5vl, llama3.2-vision)"));
                }
                Some(Err(e)) => {
                    ui.colored_label(super::theme::BAD, e);
                    ui.small(t!("Check that Ollama is running. You can get it from ollama.com."));
                }
                _ => {}
            }
            ui.horizontal(|ui| {
                ui.label(t!("Model"));
                let list = self.tagger.models.as_ref().and_then(|m| m.as_ref().ok()).cloned().unwrap_or_default();
                let current = if self.settings.local_ai.model.is_empty() { t!("(not chosen)").to_owned() } else { self.settings.local_ai.model.clone() };
                egui::ComboBox::from_id_salt("local-ai-model").width(220.0).selected_text(current).show_ui(ui, |ui| {
                    for name in list {
                        if ui.selectable_label(self.settings.local_ai.model == name, &name).clicked() {
                            self.settings.local_ai.model = name;
                            changed = true;
                        }
                    }
                });
            });
            changed |= ui.checkbox(&mut self.settings.local_ai.auto, t!("Name and tag new assets when they are imported")).changed();
            ui.add_space(4.0);
            ui.small(egui::RichText::new(t!("To name the assets you already have, use \"Name and tag\" on the Assets screen.")).color(super::theme::MUTED));
            if let Some(e) = &self.tagger.error {
                ui.colored_label(super::theme::BAD, e);
            }
        });
        if changed {
            let _ = self.settings.save();
        }
        self.tagger.open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_cleaned() {
        let got = parse_answer("```json\n{\"name\": \"「金色の龍」\", \"tags\": [\"龍\", \" 金 \", \"龍\", \"\", \"#和柄\"]}\n```").unwrap();
        assert_eq!(got.label.as_deref(), Some("金色の龍"));
        assert_eq!(got.tags, ["龍", "金", "和柄"]);
        assert!(parse_answer("{\"name\": \"\", \"tags\": []}").is_err());
        assert!(parse_answer("ごめんなさい").is_err());
    }

    #[test]
    fn pictures_are_shrunk_for_the_ai() {
        let rgba = vec![255u8; 1000 * 500 * 4];
        let png = png_for_ai(1000, 500, &rgba, 448).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (448, 224));
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    /// Ollama の代わりに、決まった答えを返す小さなサーバー。受け取った体を返す。
    fn fake_ollama(answer: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = stream.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got);
                if let Some(head) = text.find("\r\n\r\n") {
                    let length = text[..head].lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                    if got.len() >= head + 4 + length {
                        break;
                    }
                }
            }
            let body = serde_json::json!({ "message": { "role": "assistant", "content": answer } }).to_string();
            write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&got).into_owned()
        });
        (url, handle)
    }

    #[test]
    fn assets_get_names_and_tags_from_ollama() {
        let (url, server) = fake_ollama("{\"name\": \"赤い丸\", \"tags\": [\"丸\", \"赤\"]}");
        let mut app = App::new(&super::super::Initial::default());
        app.settings.local_ai = LocalAi { url, model: "gemma3".into(), auto: true };
        // 赤い丸の PNG を素材として取り込む。
        let dir = std::env::temp_dir().join(format!("pkn-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("IMG_0001 (2).png");
        let rgba: Vec<u8> = (0..32 * 32).flat_map(|i| if (i % 32 - 16) * (i % 32 - 16) + (i / 32 - 16) * (i / 32 - 16) < 100 { [220, 30, 30, 255] } else { [0, 0, 0, 0] }).collect();
        let mut png = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png, 32, 32);
            e.set_color(png::ColorType::Rgba);
            e.write_header().unwrap().write_image_data(&rgba).unwrap();
        }
        std::fs::write(&path, png).unwrap();
        let index = app.import_image(&path, true).unwrap();
        // 答えが来るまでは、ファイルの名前を整えたもの。
        assert_eq!(app.asset_title(index), "IMG 0001");
        let request = server.join().unwrap();
        assert!(request.contains("\"model\":\"gemma3\"") && request.contains("\"images\":[\"iVBOR"), "{}", &request[..request.len().min(400)]);
        let ctx = eframe::egui::Context::default();
        for _ in 0..100 {
            app.poll_tags(&ctx);
            if app.tagger.waiting.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(app.asset_title(index), "赤い丸");
        assert_eq!(app.assets[index].stored.as_ref().unwrap().tags, ["丸", "赤"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 本物の Ollama に頼む（手元に Ollama と画像を見られるモデルがあるときだけ。`PKN_OLLAMA_MODEL=gemma3 cargo test -- --ignored ollama`）。
    #[test]
    #[ignore]
    fn ollama() {
        let model = std::env::var("PKN_OLLAMA_MODEL").unwrap_or_else(|_| "gemma3".into());
        let mut rgba = vec![0u8; 64 * 64 * 4];
        for (i, p) in rgba.chunks_mut(4).enumerate() {
            let (x, y) = ((i % 64) as i32 - 32, (i / 64) as i32 - 32);
            if x * x + y * y < 600 {
                p.copy_from_slice(&[220, 30, 30, 255]);
            }
        }
        let png = png_for_ai(64, 64, &rgba, 448).unwrap();
        let got = ask(&Job { target: Target::File("x".into()), png, url: default_url(), model }).unwrap();
        println!("{got:?}");
    }
}
