//! エージェントの欄（画面のいちばん右）。Anthropic か OpenAI の API につなぎ、対話しながら案件を直す。
//!
//! - モデルには、MCP と同じ 5 つの道具（コマンドの一覧、実行、まとめて実行、状態、見た目）を渡す。
//! - API とのやりとりは別のスレッドで行う。道具の呼び出しは、画面のスレッドで実行する（案件は画面のスレッドにある）。
//! - 1 回の依頼で変えたことは、元に戻す 1 回ぶんにまとめる。操作の記録には `agent:anthropic` のように残す。
//! - API のキーは、OS の鍵の保管庫（Windows の資格情報マネージャー、macOS のキーチェーン）に入れる。設定のファイルには書かない。
//!   保管庫にないときは、環境変数 `ANTHROPIC_API_KEY`、`OPENAI_API_KEY` も見る。
//!
//! 詳しくは docs/agent-and-mcp.md の 3 章。

use eframe::egui;
use pakenoba_i18n::t;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;

use super::App;

/// 1 回の依頼で、モデルとやりとりする回数の上限（道具を呼び続けて止まらないのを防ぐ）。
const MAX_STEPS: usize = 30;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Provider {
    Anthropic,
    OpenAi,
}

impl Provider {
    const ALL: [Provider; 2] = [Provider::Anthropic, Provider::OpenAi];

    fn name(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic",
            Provider::OpenAi => "OpenAI",
        }
    }

    /// 鍵の保管庫と、記録に使う名前。
    fn key(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
        }
    }

    /// キーのはじめの文字（入れたキーが合っていそうかを見るため。違っても止めない）。
    fn key_prefix(self) -> &'static str {
        match self {
            Provider::Anthropic => "sk-ant-",
            Provider::OpenAi => "sk-",
        }
    }

    fn env(self) -> &'static str {
        match self {
            Provider::Anthropic => "ANTHROPIC_API_KEY",
            Provider::OpenAi => "OPENAI_API_KEY",
        }
    }

    /// 選べるモデル（はじめのものが既定）。欄で別の名前も打てる。
    fn models(self) -> &'static [&'static str] {
        match self {
            Provider::Anthropic => &["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"],
            Provider::OpenAi => &["gpt-6.2-sol", "gpt-6.2-astra"],
        }
    }
}

/// キーを取り出す。保管庫、なければ環境変数。
fn load_key(provider: Provider) -> Option<String> {
    if !cfg!(test)
        && let Ok(entry) = keyring::Entry::new("PAKENOBA", provider.key())
        && let Ok(key) = entry.get_password()
        && !key.trim().is_empty()
    {
        return Some(key);
    }
    std::env::var(provider.env()).ok().filter(|k| !k.trim().is_empty())
}

fn store_key(provider: Provider, key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new("PAKENOBA", provider.key()).map_err(|e| e.to_string())?;
    entry.set_password(key.trim()).map_err(|e| e.to_string())
}

fn forget_key(provider: Provider) -> Result<(), String> {
    let entry = keyring::Entry::new("PAKENOBA", provider.key()).map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// API へ送る口。試験では、決まった答えを返すものに差し替える。
pub(super) trait Transport: Send {
    fn post(&self, url: &str, headers: &[(&str, String)], body: &Value) -> Result<Value, String>;
}

struct Http;

impl Transport for Http {
    fn post(&self, url: &str, headers: &[(&str, String)], body: &Value) -> Result<Value, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(std::time::Duration::from_secs(300))).build().into();
        let mut request = agent.post(url).header("content-type", "application/json");
        for (k, v) in headers {
            request = request.header(*k, v);
        }
        let mut response = request.send(body.to_string()).map_err(|e| t!("Could not reach the API: {e}"))?;
        let status = response.status().as_u16();
        let text = response.body_mut().with_config().limit(32 << 20).read_to_string().map_err(|e| t!("Could not read the answer: {e}"))?;
        let value: Value = serde_json::from_str(&text).map_err(|_| t!("The answer was not JSON (status {status})"))?;
        if status >= 400 {
            let message = value.pointer("/error/message").and_then(Value::as_str).unwrap_or(&text);
            return Err(t!("The API returned an error ({status}): {message}"));
        }
        Ok(value)
    }
}

/// 道具の呼び出し 1 つ（モデルが付けた番号、道具の名前、引数）。
#[derive(Debug, Clone)]
pub(super) struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

/// API のスレッドから、画面のスレッドへ。
pub(super) enum FromWorker {
    Text(String),
    Tools(Vec<ToolCall>),
    /// 終わった。会話の続き（次の依頼で、前の流れを覚えておくため）。
    Done(Vec<Value>),
    Failed(String, Vec<Value>),
}

/// API に送る依頼の中身（つなぎ先、モデル、キー、はじめの指示）。
pub(super) struct Request {
    pub provider: Provider,
    pub model: String,
    pub key: String,
    pub system: String,
}

/// API のスレッドの仕事: 依頼から、道具の呼び出しを繰り返して、答えが出るまで。
pub(super) fn run_worker(transport: &dyn Transport, request: &Request, mut messages: Vec<Value>, to_ui: &Sender<FromWorker>, results: &Receiver<Vec<(String, Result<Value, String>)>>, cancel: &AtomicBool) {
    let Request { provider, model, key, system } = request;
    let provider = *provider;
    let tools = super::mcp::tools();
    for _ in 0..MAX_STEPS {
        if cancel.load(Ordering::Relaxed) {
            let _ = to_ui.send(FromWorker::Failed(t!("Stopped").to_owned(), messages));
            return;
        }
        let answer = match provider {
            Provider::Anthropic => {
                let tools: Vec<Value> = tools.as_array().unwrap().iter().map(|t| json!({ "name": t["name"], "description": t["description"], "input_schema": t["inputSchema"] })).collect();
                let body = json!({ "model": model, "max_tokens": 8192, "system": system, "tools": tools, "messages": messages });
                transport.post("https://api.anthropic.com/v1/messages", &[("x-api-key", key.to_owned()), ("anthropic-version", "2023-06-01".to_owned())], &body)
            }
            Provider::OpenAi => {
                let tools: Vec<Value> = tools.as_array().unwrap().iter().map(|t| json!({ "type": "function", "function": { "name": t["name"], "description": t["description"], "parameters": t["inputSchema"] } })).collect();
                let mut all = vec![json!({ "role": "system", "content": system })];
                all.extend(messages.iter().cloned());
                let body = json!({ "model": model, "tools": tools, "messages": all });
                transport.post("https://api.openai.com/v1/chat/completions", &[("authorization", format!("Bearer {key}"))], &body)
            }
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(e) => {
                let _ = to_ui.send(FromWorker::Failed(e, messages));
                return;
            }
        };
        // 答えから、文字と道具の呼び出しを取り出し、会話に足す。
        let (texts, calls) = match provider {
            Provider::Anthropic => {
                let content = answer.get("content").cloned().unwrap_or(json!([]));
                let blocks = content.as_array().cloned().unwrap_or_default();
                let texts: Vec<String> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str().map(str::to_owned)).collect();
                let calls: Vec<ToolCall> = blocks.iter().filter(|b| b["type"] == "tool_use").map(|b| ToolCall { id: b["id"].as_str().unwrap_or("").to_owned(), name: b["name"].as_str().unwrap_or("").to_owned(), args: b["input"].clone() }).collect();
                messages.push(json!({ "role": "assistant", "content": content }));
                (texts, calls)
            }
            Provider::OpenAi => {
                let message = answer.pointer("/choices/0/message").cloned().unwrap_or(json!({ "role": "assistant", "content": "" }));
                let texts: Vec<String> = message["content"].as_str().filter(|s| !s.trim().is_empty()).map(|s| vec![s.to_owned()]).unwrap_or_default();
                let calls: Vec<ToolCall> = message["tool_calls"].as_array().cloned().unwrap_or_default().iter().map(|c| ToolCall { id: c["id"].as_str().unwrap_or("").to_owned(), name: c["function"]["name"].as_str().unwrap_or("").to_owned(), args: c["function"]["arguments"].as_str().and_then(|a| serde_json::from_str(a).ok()).unwrap_or(json!({})) }).collect();
                messages.push(message);
                (texts, calls)
            }
        };
        for text in texts {
            let _ = to_ui.send(FromWorker::Text(text));
        }
        if calls.is_empty() {
            let _ = to_ui.send(FromWorker::Done(messages));
            return;
        }
        if to_ui.send(FromWorker::Tools(calls.clone())).is_err() {
            return;
        }
        let Ok(done) = results.recv() else { return };
        // 道具の結果を、会話に足す。絵は、絵として渡す。
        match provider {
            Provider::Anthropic => {
                let blocks: Vec<Value> = done
                    .into_iter()
                    .map(|(id, result)| {
                        let (content, error) = match result {
                            Ok(mut value) => {
                                let mut parts = Vec::new();
                                if let Some(png) = value.get("png_base64").and_then(Value::as_str).map(str::to_owned) {
                                    parts.push(json!({ "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": png } }));
                                    value.as_object_mut().map(|m| m.remove("png_base64"));
                                }
                                parts.push(json!({ "type": "text", "text": value.to_string() }));
                                (Value::Array(parts), false)
                            }
                            Err(e) => (json!([{ "type": "text", "text": e }]), true),
                        };
                        json!({ "type": "tool_result", "tool_use_id": id, "content": content, "is_error": error })
                    })
                    .collect();
                messages.push(json!({ "role": "user", "content": blocks }));
            }
            Provider::OpenAi => {
                let mut images = Vec::new();
                for (id, result) in done {
                    let content = match result {
                        Ok(mut value) => {
                            if let Some(png) = value.get("png_base64").and_then(Value::as_str).map(str::to_owned) {
                                images.push(json!({ "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{png}") } }));
                                value.as_object_mut().map(|m| m.remove("png_base64"));
                            }
                            value.to_string()
                        }
                        Err(e) => format!("Error: {e}"),
                    };
                    messages.push(json!({ "role": "tool", "tool_call_id": id, "content": content }));
                }
                // 道具の答えには絵を入れられないので、続けて、使う人の言葉として絵を渡す。
                if !images.is_empty() {
                    let mut content = vec![json!({ "type": "text", "text": "Images returned by render_preview:" })];
                    content.extend(images);
                    messages.push(json!({ "role": "user", "content": content }));
                }
            }
        }
    }
    let _ = to_ui.send(FromWorker::Failed(t!("Stopped after {MAX_STEPS} steps").to_owned(), messages));
}

/// 動いている依頼。
struct Job {
    from: Receiver<FromWorker>,
    to: Sender<Vec<(String, Result<Value, String>)>>,
    cancel: Arc<AtomicBool>,
    /// この依頼で、もう元に戻す記録を取ったか。
    recorded: bool,
}

/// やりとり 1 回ぶん（使う人の依頼と、その答え）。
pub(super) struct Turn {
    pub request: String,
    pub replies: Vec<String>,
    /// 変えたこと（画面に出す言葉と、対象のレイヤーの印）。
    pub changes: Vec<(String, Vec<u32>)>,
    pub error: Option<String>,
    pub running: bool,
    /// 終わったときの、元に戻す記録の数（このあとにほかの変更がなければ、この依頼を元に戻せる）。
    pub history_after: Option<usize>,
}

/// キーを入れる窓の状態。
pub(super) struct KeyDialog {
    provider: Provider,
    /// 打っている文字（閉じたら消す）。
    text: String,
    /// 伏せ字をやめて見せている。
    reveal: bool,
    /// 開いたばかり（欄に入力を向ける）。
    first: bool,
}

impl KeyDialog {
    fn new(provider: Provider) -> Self {
        KeyDialog { provider, text: String::new(), reveal: false, first: true }
    }
}

/// エージェントの欄の状態（タブをまたいで 1 つ。依頼は、送ったときのタブの案件に効く）。
pub(super) struct Agent {
    pub open: bool,
    provider: Provider,
    model: [String; 2],
    input: String,
    turns: Vec<Turn>,
    /// 会話の続き（API に送る形）。つなぎ先を変えたら捨てる。
    messages: Vec<Value>,
    job: Option<Job>,
    /// キーを入れる窓。
    key_dialog: Option<KeyDialog>,
    /// 試験で差し替える口。
    transport: Option<Box<dyn Fn() -> Box<dyn Transport> + Send>>,
}

impl Default for Agent {
    fn default() -> Self {
        Agent { open: false, provider: Provider::Anthropic, model: Provider::ALL.map(|p| p.models()[0].to_owned()), input: String::new(), turns: Vec::new(), messages: Vec::new(), job: None, key_dialog: None, transport: None }
    }
}

/// 道具の呼び出しから、「変えたこと」の言葉と対象を作る（読むだけのものは出さない）。
fn changes_of(call: &ToolCall) -> Vec<(String, Vec<u32>)> {
    let one = |name: &str, args: &Value| -> Option<(String, Vec<u32>)> {
        let info = super::commands::info(name)?;
        if info.read_only {
            return None;
        }
        let ids: Vec<u32> = match (args.get("ids"), args.get("id")) {
            (Some(Value::Array(list)), _) => list.iter().filter_map(|v| v.as_u64().map(|v| v as u32)).collect(),
            (_, Some(id)) => id.as_u64().map(|v| vec![v as u32]).unwrap_or_default(),
            _ => Vec::new(),
        };
        Some((super::oplog::command_label(name), ids))
    };
    match call.name.as_str() {
        "command_run" => call.args["name"].as_str().and_then(|n| one(n, &call.args["args"])).into_iter().collect(),
        "command_batch" => call.args["commands"].as_array().map(|list| list.iter().filter_map(|c| one(c["name"].as_str()?, &c["args"])).collect()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

impl App {
    fn actor_name(&self) -> String {
        format!("agent:{}", self.agent.provider.key())
    }

    /// 依頼を送る。
    fn send_request(&mut self, ctx: &egui::Context) {
        let request = self.agent.input.trim().to_owned();
        if request.is_empty() || self.agent.job.is_some() {
            return;
        }
        let provider = self.agent.provider;
        let Some(key) = load_key(provider) else {
            self.agent.key_dialog = Some(KeyDialog::new(provider));
            return;
        };
        self.agent.input.clear();
        let doc = self.run_command("person", "doc.inspect", Value::Null).unwrap_or_default();
        let language = pakenoba_i18n::current();
        let system = format!("{}\nYou are the assistant inside the PAKENOBA app. Reply in the user's language (the app is set to \"{language}\"). Keep replies short and say what you changed. Save or export only when the user asks.", super::mcp::INSTRUCTIONS);
        let content = format!("{request}\n\n[The project now (doc_inspect)]\n{doc}");
        let mut messages = std::mem::take(&mut self.agent.messages);
        messages.push(json!({ "role": "user", "content": content }));
        let (to_ui, from) = channel();
        let (to, results) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let model = self.agent.model[provider as usize].clone();
        let transport: Box<dyn Transport> = match &self.agent.transport {
            Some(make) => make(),
            None => Box::new(Http),
        };
        let wake = ctx.clone();
        std::thread::spawn(move || {
            // 知らせるたびに、画面を起こす。
            let (relay, relayed) = channel();
            let waker = std::thread::spawn(move || {
                for message in relayed {
                    if to_ui.send(message).is_err() {
                        break;
                    }
                    wake.request_repaint();
                }
            });
            let request = Request { provider, model, key, system };
            run_worker(transport.as_ref(), &request, messages, &relay, &results, &stop);
            drop(relay);
            let _ = waker.join();
        });
        self.agent.turns.push(Turn { request, replies: Vec::new(), changes: Vec::new(), error: None, running: true, history_after: None });
        self.agent.job = Some(Job { from, to, cancel, recorded: false });
    }

    /// API のスレッドからの知らせを受ける。道具の呼び出しは、ここで実行する。毎フレーム呼ぶ。
    pub(super) fn poll_agent(&mut self) {
        let Some(job) = &mut self.agent.job else { return };
        let mut finished = None;
        let mut calls = Vec::new();
        while let Ok(message) = job.from.try_recv() {
            match message {
                FromWorker::Text(text) => {
                    if let Some(turn) = self.agent.turns.last_mut() {
                        turn.replies.push(text);
                    }
                }
                FromWorker::Tools(list) => calls = list,
                FromWorker::Done(messages) => finished = Some((messages, None)),
                FromWorker::Failed(e, messages) => finished = Some((messages, Some(e))),
            }
            if !calls.is_empty() || finished.is_some() {
                break;
            }
        }
        if !calls.is_empty() {
            let actor = self.actor_name();
            let recorded = self.agent.job.as_ref().is_some_and(|j| j.recorded);
            // この依頼の変更は、元に戻す 1 回ぶんにまとめる。
            self.undo_batch = Some(recorded);
            let mut results = Vec::new();
            for call in &calls {
                let result = super::mcp::call_tool(self, &actor, &call.name, &call.args);
                if result.is_ok()
                    && let Some(turn) = self.agent.turns.last_mut()
                {
                    turn.changes.extend(changes_of(call));
                }
                results.push((call.id.clone(), result));
            }
            let recorded = self.undo_batch == Some(true);
            self.undo_batch = None;
            if let Some(job) = &mut self.agent.job {
                job.recorded = recorded;
                let _ = job.to.send(results);
            }
        }
        if let Some((messages, error)) = finished {
            let recorded = self.agent.job.as_ref().is_some_and(|j| j.recorded);
            self.agent.job = None;
            self.agent.messages = messages;
            let depth = self.history.depth();
            if let Some(turn) = self.agent.turns.last_mut() {
                turn.running = false;
                turn.error = error;
                turn.history_after = recorded.then_some(depth);
            }
        }
    }

    /// 右の端の、エージェントの欄。
    pub(super) fn agent_panel(&mut self, ui: &mut egui::Ui) {
        if !self.agent.open {
            return;
        }
        egui::Panel::right("agent").resizable(true).default_size(340.0).size_range(280.0..=560.0).frame(super::theme::side_frame()).show(ui, |ui| {
            super::panels::boxed(ui, t!("Agent"), true, |ui| self.agent_body(ui));
        });
        self.key_window(ui.ctx());
    }

    fn agent_body(&mut self, ui: &mut egui::Ui) {
        let busy = self.agent.job.is_some();
        // つなぎ先とモデル。
        ui.horizontal(|ui| {
            let mut provider = self.agent.provider;
            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("agent-provider").width(100.0).selected_text(provider.name()).show_ui(ui, |ui| {
                    for p in Provider::ALL {
                        ui.selectable_value(&mut provider, p, p.name());
                    }
                });
                let models = provider.models();
                let model = &mut self.agent.model[provider as usize];
                egui::ComboBox::from_id_salt("agent-model").width(ui.available_width() - 4.0).selected_text(model.as_str()).show_ui(ui, |ui| {
                    for m in models {
                        ui.selectable_value(model, (*m).to_owned(), *m);
                    }
                    ui.separator();
                    ui.add(egui::TextEdit::singleline(model).hint_text(t!("Other model name")));
                });
            });
            if provider != self.agent.provider {
                // つなぎ先を変えたら、会話の続きは捨てる（形が違うため）。
                self.agent.provider = provider;
                self.agent.messages.clear();
            }
        });
        let provider = self.agent.provider;
        ui.horizontal(|ui| {
            let has_key = load_key(provider).is_some();
            if has_key {
                ui.label(egui::RichText::new(t!("API key: saved")).color(super::theme::MUTED));
                if ui.small_button(t!("Change…")).clicked() {
                    self.agent.key_dialog = Some(KeyDialog::new(provider));
                }
                if ui.small_button(t!("Forget")).on_hover_text(t!("Remove the key from the OS keychain")).clicked() {
                    match forget_key(provider) {
                        Ok(()) => self.say(t!("Removed the {} API key from the OS keychain.", provider.name())),
                        Err(e) => self.say(e),
                    }
                }
            } else if ui.button(t!("Enter the API key…")).clicked() {
                self.agent.key_dialog = Some(KeyDialog::new(provider));
            }
        });
        ui.small(egui::RichText::new(t!("Sent to {}: your request, the project's state as text, and preview pictures of the dieline when the model asks for them. Asset files are not sent.", provider.name())).color(super::theme::FAINT));
        ui.separator();

        // やりとり。
        let input_h = 92.0;
        let mut select = None;
        let mut undo = false;
        let depth = self.history.depth();
        let last = self.agent.turns.len().saturating_sub(1);
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).max_height((ui.available_height() - input_h).max(80.0)).show(ui, |ui| {
            if self.agent.turns.is_empty() {
                ui.weak(t!("Ask for changes in your own words, for example \"Make the logo on the front a little bigger and turn the band gold\". Each request can be undone in one step."));
            }
            for (k, turn) in self.agent.turns.iter().enumerate() {
                egui::Frame::new().fill(super::theme::SELECTED).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(&turn.request);
                });
                for reply in &turn.replies {
                    ui.label(reply);
                }
                if !turn.changes.is_empty() {
                    ui.add_space(2.0);
                    for (label, ids) in &turn.changes {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new("•").color(super::theme::ACCENT));
                            ui.label(label);
                            for id in ids {
                                if ui.link(t!("layer {}", id)).clicked() {
                                    select = Some(*id);
                                }
                            }
                        });
                    }
                }
                if turn.running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak(t!("Working…"));
                    });
                }
                if let Some(e) = &turn.error {
                    ui.colored_label(egui::Color32::from_rgb(240, 120, 100), e);
                }
                if k == last && turn.history_after == Some(depth) && ui.small_button(t!("Undo this request")).clicked() {
                    undo = true;
                }
                ui.add_space(8.0);
            }
        });
        if let Some(id) = select
            && let Ok(index) = self.index_of(i64::from(id))
        {
            self.select_only(Some(index));
        }
        if undo {
            self.undo();
            if let Some(turn) = self.agent.turns.last_mut() {
                turn.history_after = None;
                turn.replies.push(t!("(Undone)").to_owned());
            }
        }

        // 入力。
        ui.separator();
        let response = ui.add_enabled(!busy, egui::TextEdit::multiline(&mut self.agent.input).desired_rows(3).desired_width(f32::INFINITY).hint_text(t!("Ask the agent… (Ctrl+Enter to send)")));
        let send_key = response.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            if busy {
                if ui.button(t!("Stop")).clicked()
                    && let Some(job) = &self.agent.job
                {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            } else if ui.add_enabled(!self.agent.input.trim().is_empty(), egui::Button::new(t!("Send"))).clicked() || send_key {
                self.send_request(ui.ctx());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!busy && !self.agent.turns.is_empty(), egui::Button::new(t!("New conversation")).small()).clicked() {
                    self.agent.turns.clear();
                    self.agent.messages.clear();
                }
            });
        });
    }

    /// API のキーを入れる窓（PhotoCraft のパスワードの窓と同じ形: 題の帯、伏せ字の欄と見る切り替え、右にそろえたボタン）。
    /// 入れたキーは、OS の鍵の保管庫にしまう。打った文字は、窓を閉じたら消す（メモリーにも残さない）。
    fn key_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.agent.key_dialog.take() else { return };
        let provider = dialog.provider;
        let typed = !dialog.text.trim().is_empty();
        let looks_right = dialog.text.trim().starts_with(provider.key_prefix());
        let title = t!("{} API key", provider.name());
        let buttons = [super::dialog::Button::accept(t!("Save"), typed), super::dialog::Button::cancel(t!("Cancel"))];
        let picked = super::dialog::show(ctx, "agent-key", &title, 420.0, &buttons, |ui| {
            ui.label(t!("The key is kept in the OS keychain (Windows Credential Manager, macOS Keychain), not in a settings file."));
            ui.add_space(2.0);
            ui.label(egui::RichText::new(t!("API key")).color(super::theme::MUTED));
            ui.horizontal(|ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut dialog.text).password(!dialog.reveal).desired_width(ui.available_width() - 34.0).hint_text(provider.key_prefix()));
                if dialog.first {
                    field.request_focus();
                    dialog.first = false;
                }
                let tip = if dialog.reveal { t!("Hide the key") } else { t!("Show the key") };
                if super::icons::eye_button(ui, dialog.reveal).on_hover_text(tip).clicked() {
                    dialog.reveal = !dialog.reveal;
                    field.request_focus();
                }
            });
            if typed && !looks_right {
                ui.colored_label(egui::Color32::from_rgb(230, 170, 90), t!("{} keys usually start with \"{}\". Check that this is the right key.", provider.name(), provider.key_prefix()));
            } else {
                ui.small(egui::RichText::new(t!("Create a key in the {} developer console and paste it here.", provider.name())).color(super::theme::FAINT));
            }
        });
        match picked.map(|i| buttons[i].role) {
            Some(super::dialog::Role::Accept) => {
                match store_key(provider, &dialog.text) {
                    Ok(()) => self.say(t!("Saved the {} API key in the OS keychain.", provider.name())),
                    Err(e) => self.say(t!("Could not save the key in the OS keychain: {e}")),
                }
                zeroize::Zeroize::zeroize(&mut dialog.text);
            }
            Some(super::dialog::Role::Cancel) => zeroize::Zeroize::zeroize(&mut dialog.text),
            None => self.agent.key_dialog = Some(dialog),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Initial;
    use super::*;
    use std::sync::Mutex;

    /// 決まった答えを順に返し、送られた中身を覚える口。
    struct Script {
        answers: Arc<Mutex<Vec<Value>>>,
        sent: Arc<Mutex<Vec<Value>>>,
    }

    impl Transport for Script {
        fn post(&self, _url: &str, _headers: &[(&str, String)], body: &Value) -> Result<Value, String> {
            self.sent.lock().unwrap().push(body.clone());
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() { Err("no more answers".to_owned()) } else { Ok(answers.remove(0)) }
        }
    }

    fn run(provider: Provider, answers: Vec<Value>) -> (App, Vec<Value>) {
        let mut app = App::new(&Initial::default());
        app.decals.clear();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (a, s) = (Arc::new(Mutex::new(answers)), sent.clone());
        app.agent.transport = Some(Box::new(move || Box::new(Script { answers: a.clone(), sent: s.clone() })));
        app.agent.provider = provider;
        app.agent.input = "正面にロゴを置いて".to_owned();
        // SAFETY: 試験のスレッドの中で、キーを環境変数に置く（ほかの試験は、この名前を読まない）。
        unsafe { std::env::set_var(provider.env(), "test-key") };
        app.send_request(&egui::Context::default());
        let start = std::time::Instant::now();
        while app.agent.job.is_some() && start.elapsed().as_secs() < 10 {
            app.poll_agent();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let sent = sent.lock().unwrap().clone();
        (app, sent)
    }

    #[test]
    fn anthropicの道具の呼び出しで柄を置き_1回で元に戻せる() {
        let answers = vec![
            json!({ "content": [{ "type": "text", "text": "置きます。" }, { "type": "tool_use", "id": "t1", "name": "command_batch", "input": { "commands": [{ "name": "decal.add", "args": { "builtin": "logo" } }, { "name": "layer.opacity.set", "args": { "ids": [1], "opacity": 80 } }] } }], "stop_reason": "tool_use" }),
            json!({ "content": [{ "type": "tool_use", "id": "t2", "name": "render_preview", "input": { "max": 200 } }], "stop_reason": "tool_use" }),
            json!({ "content": [{ "type": "text", "text": "正面にロゴを置きました。" }], "stop_reason": "end_turn" }),
        ];
        let (mut app, sent) = run(Provider::Anthropic, answers);
        let turn = &app.agent.turns[0];
        assert_eq!(turn.replies, ["置きます。", "正面にロゴを置きました。"]);
        assert!(turn.error.is_none(), "{:?}", turn.error);
        assert_eq!(turn.changes.len(), 2);
        assert_eq!((app.decals.len(), app.decals[0].layer.opacity), (1, 80));
        assert!(app.log.iter().all(|e| e.actor == "agent:anthropic"));
        // 道具の結果は tool_result で返し、見た目は絵として渡す。
        let last = &sent[2]["messages"];
        let result = &last[last.as_array().unwrap().len() - 1]["content"][0];
        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["content"][0]["type"], "image");
        assert_eq!(sent[0]["tools"].as_array().unwrap().len(), 5);
        // 1 回の依頼は、1 回で元に戻る。
        assert_eq!(turn.history_after, Some(app.history.depth()));
        app.undo();
        assert!(app.decals.is_empty());
        // 会話の続きを覚えている。
        assert_eq!(app.agent.messages.len(), 6);
    }

    #[test]
    fn openaiでも同じように動き_失敗は欄に出る() {
        let answers = vec![
            json!({ "choices": [{ "message": { "role": "assistant", "content": null, "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "command_run", "arguments": "{\"name\":\"decal.add\",\"args\":{\"shape\":{\"kind\":\"ellipse\"}}}" } }] } }] }),
        ];
        let (app, sent) = run(Provider::OpenAi, answers);
        assert_eq!(app.decals.len(), 1);
        assert!(app.log.iter().all(|e| e.actor == "agent:openai"));
        assert_eq!(sent[0]["messages"][0]["role"], "system");
        assert_eq!(sent[0]["tools"][0]["type"], "function");
        // 2 回目の答えがないので、失敗として欄に出る。
        assert!(app.agent.turns[0].error.is_some());
        assert_eq!(sent[1]["messages"].as_array().unwrap().last().unwrap()["role"], "tool");
    }
}
