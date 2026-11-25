//! 利用状況を送る（同意した人だけ。Aptabase へ、匿名で）。
//!
//! - 送り先の鍵（App Key）は、ビルドのときに環境変数 `PAKENOBA_APTABASE_KEY` で入れる。入れずにビルドしたもの
//!   （人が自分でビルドしたものや、改造して配るもの）は、何も送らず、同意も聞かない。
//! - はじめて起動したときに、送ってよいかを聞く。はじめは「送らない」。「編集」→「環境設定」でいつでも切れる。
//! - 送るのは、自分で決めた少しのイベントだけ（起動、型を決めた、書き出した、AI の機能を使った）。
//!   案件の中身、ファイルの名前、絵、文字は送らない。Aptabase は、版、OS、言語、国（IP から。IP は残さない）を付ける。
//! - 自分についての答え（職業と年代）は、別に同意したときだけ、起動のイベントに付ける。どちらも「答えない」を選べる。
//! - 送ったもの（と、送る前のもの）は、「送る内容を見る」で、そのまま見せる。
//! - 送るのは別のスレッドで、失敗しても、アプリの動きには関わらない（その回のぶんは捨てる）。
//!
//! 送り方は、Aptabase の「自分で SDK を作るとき」の決まり: `POST https://{eu|us}.aptabase.com/api/v0/events`、
//! 見出し `App-Key`、中身はイベントの配列（1 回に 25 個まで）。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui;
use pakenoba_i18n::t;

/// ビルドのときに入れた鍵。
const BUILD_KEY: Option<&str> = option_env!("PAKENOBA_APTABASE_KEY");

/// 見せるために覚えておく、最近のイベントの数。
const KEEP: usize = 50;

/// 同意と、自分についての答え（設定に残す）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Consent {
    /// 聞いたか（聞いたら、もう聞かない）。
    #[serde(default)]
    pub asked: bool,
    /// 利用状況を送る。
    #[serde(default)]
    pub send: bool,
    /// 自分についての答えも送る。
    #[serde(default)]
    pub send_profile: bool,
    /// 職業（`JOBS` の名前。答えないなら空）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub job: String,
    /// 年代（`AGES` の名前。答えないなら空）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub age: String,
}

/// 職業の分類（送る名前）。
pub const JOBS: [&str; 6] = ["designer", "print", "brand", "student", "hobby", "other"];

/// 職業の分類の、画面の名前。
fn job_label(key: &str) -> &'static str {
    match key {
        "designer" => t!("Designer (graphic, package)"),
        "print" => t!("Printing or carton making"),
        "brand" => t!("Brand owner or marketing"),
        "student" => t!("Student or teacher"),
        "hobby" => t!("Hobby"),
        "other" => t!("Other"),
        _ => t!("Prefer not to say"),
    }
}

/// 年代。
pub const AGES: [&str; 6] = ["10s", "20s", "30s", "40s", "50s", "60+"];

/// 送る係。
pub struct Usage {
    key: Option<String>,
    session: String,
    queue: Option<Sender<Value>>,
    /// 最近のイベント（送ったかどうかに関わらず。見せるため）。
    pub recent: Arc<Mutex<Vec<Value>>>,
    /// 同意の窓を開いている。
    pub ask_open: bool,
    /// 送る内容の窓を開いている。
    pub view_open: bool,
    /// 最後に送った型（同じ型を続けて送らない）。
    last_shape: Option<String>,
}

impl Default for Usage {
    fn default() -> Self {
        Usage::new(None)
    }
}

/// 鍵から、送り先。
fn host(key: &str) -> Option<&'static str> {
    if key.starts_with("A-EU-") {
        Some("https://eu.aptabase.com")
    } else if key.starts_with("A-US-") {
        Some("https://us.aptabase.com")
    } else {
        None
    }
}

/// 起動ごとの番号（Aptabase の決まり: 秒の時刻のあとに、8 けたの乱数）。人を見分けるものではない。
fn session_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(now.as_nanos());
    format!("{}{:08}", now.as_secs(), h.finish() % 100_000_000)
}

/// いまの時刻（UTC、ISO 8601）。
fn timestamp() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

impl Usage {
    /// `key` は送り先の鍵。`None` ならビルドのときの鍵（なければ、何も送らない）。
    pub fn new(key: Option<String>) -> Usage {
        let key = key.or_else(|| BUILD_KEY.filter(|k| !k.trim().is_empty()).map(str::to_owned)).filter(|k| host(k).is_some());
        Usage { key, session: session_id(), queue: None, recent: Arc::new(Mutex::new(Vec::new())), ask_open: false, view_open: false, last_shape: None }
    }

    /// 送れる作りか（鍵が入っているか）。
    pub fn available(&self) -> bool {
        self.key.is_some()
    }

    /// 1 つのイベントの中身（Aptabase の形）。
    fn event(&self, name: &str, props: Value) -> Value {
        let os = if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        };
        json!({
            "timestamp": timestamp(),
            "sessionId": self.session,
            "eventName": name,
            "systemProps": {
                "locale": pakenoba_i18n::current(),
                "osName": os,
                "isDebug": cfg!(debug_assertions),
                "appVersion": env!("CARGO_PKG_VERSION"),
                "sdkVersion": concat!("pakenoba@", env!("CARGO_PKG_VERSION")),
            },
            "props": props,
        })
    }

    /// イベントを 1 つ送る（同意していなければ、何もしない）。
    pub fn track(&mut self, consent: &Consent, name: &str, props: Value) {
        if !consent.send || self.key.is_none() {
            return;
        }
        let event = self.event(name, props);
        if let Ok(mut recent) = self.recent.lock() {
            recent.push(event.clone());
            let extra = recent.len().saturating_sub(KEEP);
            recent.drain(..extra);
        }
        if self.queue.is_none() {
            self.queue = self.key.clone().map(start_sender);
        }
        if let Some(queue) = &self.queue
            && queue.send(event).is_err()
        {
            self.queue = None;
        }
    }

    /// 型を決めた（前と同じ型なら送らない）。
    pub fn track_shape(&mut self, consent: &Consent, shape: String) {
        if self.last_shape.as_deref() == Some(shape.as_str()) {
            return;
        }
        let first = self.last_shape.is_none();
        self.last_shape = Some(shape.clone());
        // 起動したときの型は、型を選んだことにしない。
        if !first {
            self.track(consent, "shape_chosen", json!({ "shape": shape }));
        }
    }

    /// 起動のイベント（自分についての答えは、同意したときだけ付ける）。
    pub fn track_start(&mut self, consent: &Consent) {
        let mut props = serde_json::Map::new();
        if consent.send_profile {
            if !consent.job.is_empty() {
                props.insert("job".into(), json!(consent.job));
            }
            if !consent.age.is_empty() {
                props.insert("age".into(), json!(consent.age));
            }
        }
        self.track(consent, "app_started", Value::Object(props));
    }
}

/// 送る係のスレッド。イベントを集め、少し待ってから、25 個ずつ送る。
fn start_sender(key: String) -> Sender<Value> {
    let (send, receive) = channel::<Value>();
    std::thread::spawn(move || {
        let Some(base) = host(&key) else { return };
        let url = format!("{base}/api/v0/events");
        while let Ok(first) = receive.recv() {
            let mut batch = vec![first];
            // 続けて来るものを、少しまとめる。
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while batch.len() < 25 {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                match receive.recv_timeout(left) {
                    Ok(event) => batch.push(event),
                    Err(_) => break,
                }
            }
            let body = Value::Array(batch).to_string();
            let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(10))).build().into();
            if let Err(e) = agent.post(&url).header("content-type", "application/json").header("App-Key", &key).send(body) {
                eprintln!("Could not send usage data: {e}");
            }
        }
    });
    send
}

impl super::App {
    /// 毎回の描き直しで: はじめての起動なら同意を聞き、型が変わったら送る。
    pub(super) fn usage_frame(&mut self, ctx: &egui::Context) {
        if !self.usage.available() {
            return;
        }
        if !self.usage_started {
            self.usage_started = true;
            if self.settings.usage.asked {
                self.usage.track_start(&self.settings.usage);
            } else {
                self.usage.ask_open = true;
            }
        }
        let shape = self.usage_shape_name();
        self.usage.track_shape(&self.settings.usage, shape);
        self.usage_windows(ctx);
    }

    /// 送る型の名前（ECMA は番号、DXF は「dxf」だけ。ファイルの名前は送らない）。
    fn usage_shape_name(&self) -> String {
        let d = &self.dims;
        match d.sides {
            0 => "tuck_box".into(),
            2 => "gable".into(),
            super::PILLOW => "pillow".into(),
            super::PLAIN_PILLOW => "plain_pillow".into(),
            super::IMPORTED => "dxf".into(),
            super::ECMA => pakenoba_core::ecma::Code::from_u32(d.ecma).map_or_else(|| "ecma".into(), |c| format!("ecma:{}", c.name())),
            n => format!("prism{n}"),
        }
    }

    /// イベントを送る（同意していなければ、何もしない）。
    pub(super) fn track(&mut self, name: &str, props: Value) {
        self.usage.track(&self.settings.usage, name, props);
    }

    fn save_usage(&mut self) {
        if let Err(e) = self.settings.save() {
            self.say(e);
        }
    }

    /// 同意の窓と、送る内容の窓。
    fn usage_windows(&mut self, ctx: &egui::Context) {
        if self.usage.ask_open {
            let mut decided = None;
            egui::Window::new(t!("Help improve PAKENOBA")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).default_width(460.0).show(ctx, |ui| {
                ui.label(t!("May PAKENOBA send anonymous usage data? It helps decide what to improve."));
                ui.add_space(4.0);
                ui.label(t!("What is sent: that the app started, which box type you chose, which kind of file you exported, and which AI features you used. The service adds the app version, the OS, the language and the country (worked out from the IP address, which is not kept)."));
                ui.label(t!("What is never sent: your projects, file names, pictures or text. No account or device ID is used."));
                ui.add_space(6.0);
                ui.separator();
                ui.label(t!("If you like, tell us a little about yourself (sent with the start of the app, only if you tick the box below)."));
                self.profile_fields(ui);
                ui.checkbox(&mut self.settings.usage.send_profile, t!("Also send these answers"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("Send usage data")).clicked() {
                        decided = Some(true);
                    }
                    if ui.button(t!("Don't send")).clicked() {
                        decided = Some(false);
                    }
                });
                ui.small(t!("You can change this at any time in Edit > Preferences."));
            });
            if let Some(send) = decided {
                self.settings.usage.asked = true;
                self.settings.usage.send = send;
                self.usage.ask_open = false;
                self.save_usage();
                self.usage.track_start(&self.settings.usage);
            }
        }
        if self.usage.view_open {
            let mut open = true;
            let recent: Vec<Value> = self.usage.recent.lock().map(|r| r.clone()).unwrap_or_default();
            egui::Window::new(t!("Usage data sent")).open(&mut open).default_size([520.0, 420.0]).show(ctx, |ui| {
                ui.label(t!("The latest events this session, exactly as they are sent (newest last). Nothing is sent while sending is off."));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if recent.is_empty() {
                        ui.weak(t!("Nothing has been sent yet."));
                    }
                    for event in &recent {
                        ui.monospace(serde_json::to_string_pretty(event).unwrap_or_default());
                        ui.separator();
                    }
                });
            });
            self.usage.view_open = open;
        }
    }

    /// 職業と年代を選ぶ欄。
    fn profile_fields(&mut self, ui: &mut egui::Ui) {
        let consent = &mut self.settings.usage;
        egui::Grid::new("usage-profile").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(t!("Work"));
            let shown = job_label(&consent.job);
            egui::ComboBox::from_id_salt("usage-job").width(240.0).selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut consent.job, String::new(), t!("Prefer not to say"));
                for key in JOBS {
                    ui.selectable_value(&mut consent.job, key.to_owned(), job_label(key));
                }
            });
            ui.end_row();
            ui.label(t!("Age"));
            let shown = if consent.age.is_empty() { t!("Prefer not to say").to_owned() } else { consent.age.clone() };
            egui::ComboBox::from_id_salt("usage-age").width(240.0).selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut consent.age, String::new(), t!("Prefer not to say"));
                for age in AGES {
                    ui.selectable_value(&mut consent.age, age.to_owned(), age);
                }
            });
            ui.end_row();
        });
    }

    /// 「編集」→「環境設定」の中の、利用状況の項目。
    pub(super) fn usage_menu(&mut self, ui: &mut egui::Ui) {
        if !self.usage.available() {
            return;
        }
        ui.menu_button(t!("Usage data"), |ui| {
            ui.set_min_width(320.0);
            let mut changed = ui.checkbox(&mut self.settings.usage.send, t!("Send anonymous usage data")).changed();
            changed |= ui.checkbox(&mut self.settings.usage.send_profile, t!("Also send my answers about myself")).changed();
            let before = self.settings.usage.clone();
            self.profile_fields(ui);
            changed |= before != self.settings.usage;
            if ui.button(t!("Forget my answers")).clicked() {
                (self.settings.usage.job, self.settings.usage.age, self.settings.usage.send_profile) = (String::new(), String::new(), false);
                changed = true;
            }
            if ui.button(t!("See what is sent…")).clicked() {
                self.usage.view_open = true;
                ui.close();
            }
            if changed {
                self.settings.usage.asked = true;
                self.save_usage();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 同意していなければ送らず_同意すれば決めた形で積む() {
        let mut usage = Usage::new(Some("A-EU-0000000000".into()));
        assert!(usage.available());
        let mut consent = Consent::default();
        usage.track(&consent, "app_started", json!({}));
        assert!(usage.recent.lock().unwrap().is_empty());
        // 送り先へはつながないよう、送る係を先に止めておく（積むところだけ確かめる）。
        let (send, _receive) = channel();
        usage.queue = Some(send);
        consent.send = true;
        consent.job = "designer".into();
        consent.age = "30s".into();
        usage.track_start(&consent);
        let event = usage.recent.lock().unwrap()[0].clone();
        assert_eq!(event["eventName"], "app_started");
        assert_eq!(event["props"], json!({}), "答えは、別に同意したときだけ");
        assert_eq!(event["systemProps"]["appVersion"], env!("CARGO_PKG_VERSION"));
        assert!(event["sessionId"].as_str().unwrap().len() >= 10);
        consent.send_profile = true;
        usage.track_start(&consent);
        assert_eq!(usage.recent.lock().unwrap()[1]["props"], json!({ "job": "designer", "age": "30s" }));
        // 型は、変わったときだけ。起動したときの型は送らない。
        usage.track_shape(&consent, "tuck_box".into());
        usage.track_shape(&consent, "tuck_box".into());
        usage.track_shape(&consent, "pillow".into());
        let names: Vec<String> = usage.recent.lock().unwrap().iter().map(|e| e["eventName"].as_str().unwrap().to_owned()).collect();
        assert_eq!(names, ["app_started", "app_started", "shape_chosen"]);
    }

    #[test]
    fn 鍵がなければ_何もしない() {
        let mut usage = Usage::new(Some("not-a-key".into()));
        assert!(!usage.available() || BUILD_KEY.is_some());
        let consent = Consent { send: true, ..Consent::default() };
        if !usage.available() {
            usage.track(&consent, "app_started", json!({}));
            assert!(usage.recent.lock().unwrap().is_empty());
        }
        assert_eq!(host("A-US-1"), Some("https://us.aptabase.com"));
        assert_eq!(host("A-SH-1"), None);
    }
}
