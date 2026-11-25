//! 開いている画面を、MCP から動かす（`pakenoba mcp` が、動いている画面へつなぐ）。
//!
//! - 画面で「MCP からこの画面を動かす」を入れている間だけ、この PC の中だけの口（127.0.0.1 の空いている番号）を開く。
//! - 口の番号と合言葉（その場で作る乱数）を、データのフォルダの `mcp-window.json` に書く。読めるのは、この PC の同じ使う人だけ。
//! - `pakenoba mcp` は、このファイルを見つけると、画面へつないで、標準入出力の行をそのまま中継する。見つからなければ、画面なしで動く。
//! - 届いた依頼は、画面の 1 回の描き直しの中で、画面と同じコマンドの入口から実行する。結果はその場で画面に見え、元に戻せる。
//! - ファイルを読み書きできるのは、設定の「AI が使ってよいフォルダ」と、`pakenoba mcp --allow` で渡したフォルダの中だけ。
//!
//! 1 行目だけは中継の取り決め: `{"token": …, "allow": [フォルダ…]}` を送り、`{"ok": true}` が返れば、あとは MCP の行。

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

/// 口の番号と合言葉を書くファイルの名前（データのフォルダの中）。
const FILE: &str = "mcp-window.json";

/// 画面のスレッドで実行してもらう、1 通。
pub(super) struct Job {
    pub connection: usize,
    pub message: Value,
    /// つないだ側が `--allow` で渡したフォルダ。
    pub folders: Vec<PathBuf>,
    pub reply: Sender<Option<Value>>,
}

/// 開いている口。落とす（`Drop`）と、口を閉じてファイルを消す。
pub(super) struct Bridge {
    pub jobs: Receiver<Job>,
    stop: Arc<AtomicBool>,
    #[cfg_attr(not(test), allow(dead_code))]
    port: u16,
    file: Option<PathBuf>,
    /// いまつないでいる数。
    pub clients: Arc<AtomicUsize>,
    /// つないでいる相手ごとの名前（記録に残す `mcp:<名前>`）。
    pub actors: Arc<Mutex<std::collections::HashMap<usize, String>>>,
}

/// 推測できない合言葉（128 ビット）。OS が起動ごとに変える乱数の種を、何度か混ぜて作る。
fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for k in 0..4u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(k);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u32(std::process::id());
        out.push_str(&format!("{:08x}", h.finish() as u32));
    }
    out
}

/// 口の番号と合言葉のファイルの場所。
pub(crate) fn file() -> Option<PathBuf> {
    super::settings::data_dir().map(|dir| dir.join(FILE))
}

/// 自分だけが読めるように書く。
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    f.write_all(text.as_bytes())
}

impl Bridge {
    /// 口を開く。`file` に番号と合言葉を書く（`None` なら書かない。テスト用）。
    pub fn open(ctx: egui::Context, file: Option<PathBuf>) -> std::io::Result<Bridge> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let token = token();
        if let Some(path) = &file {
            write_private(path, &json!({ "port": port, "token": token, "pid": std::process::id() }).to_string())?;
        }
        let (send, jobs) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(AtomicUsize::new(0));
        let actors = Arc::new(Mutex::new(std::collections::HashMap::new()));
        {
            let (stop, clients) = (stop.clone(), clients.clone());
            std::thread::spawn(move || {
                let mut next = 0usize;
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            next += 1;
                            let (send, ctx, token, stop, clients) = (send.clone(), ctx.clone(), token.clone(), stop.clone(), clients.clone());
                            let connection = next;
                            std::thread::spawn(move || {
                                clients.fetch_add(1, Ordering::Relaxed);
                                ctx.request_repaint();
                                let _ = serve(stream, connection, &token, &send, &ctx, &stop);
                                clients.fetch_sub(1, Ordering::Relaxed);
                                ctx.request_repaint();
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                        Err(_) => std::thread::sleep(Duration::from_millis(200)),
                    }
                }
            });
        }
        Ok(Bridge { jobs, stop, port, file, clients, actors })
    }

    #[cfg(test)]
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(path) = &self.file {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 1 つのつながり。合言葉を確かめてから、行を画面へ渡し、答えを返す。
fn serve(stream: TcpStream, connection: usize, token: &str, send: &Sender<Job>, ctx: &egui::Context, stop: &AtomicBool) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    // 1 行目: 合言葉と、使ってよいフォルダ。10 秒のうちに来なければ切る。
    let started = std::time::Instant::now();
    loop {
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(()),
            Ok(_) => break,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if started.elapsed() > Duration::from_secs(10) || stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
            }
            Err(e) => return Err(e),
        }
    }
    let hello: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
    if hello.get("token").and_then(Value::as_str) != Some(token) {
        writeln!(writer, "{}", json!({ "ok": false, "error": "wrong token" }))?;
        return Ok(());
    }
    let folders: Vec<PathBuf> = hello.get("allow").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect()).unwrap_or_default();
    writeln!(writer, "{}", json!({ "ok": true }))?;
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                continue;
            }
            Err(e) => return Err(e),
        }
        if line.trim().is_empty() {
            continue;
        }
        let message = match serde_json::from_str::<Value>(line.trim()) {
            Ok(m) => m,
            Err(e) => {
                writeln!(writer, "{}", json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("Parse error: {e}") } }))?;
                continue;
            }
        };
        let (reply, answer) = channel();
        if send.send(Job { connection, message, folders: folders.clone(), reply }).is_err() {
            return Ok(());
        }
        ctx.request_repaint();
        // 画面が閉じられたら、答えは来ない。
        if let Ok(Some(reply)) = answer.recv() {
            writeln!(writer, "{reply}")?;
            writer.flush()?;
        }
    }
}

/// `pakenoba mcp` の側: 動いている画面があれば、つないで中継する。つなげたら、標準入力が閉じるまで戻らず `true`。
/// 画面がない（ファイルがない、つながらない、合言葉が違う）ときは、すぐに `false`。
pub(crate) fn relay(allow: &[PathBuf]) -> bool {
    let Some(path) = file() else { return false };
    let Ok(text) = std::fs::read_to_string(&path) else { return false };
    let Ok(info) = serde_json::from_str::<Value>(&text) else { return false };
    let (Some(port), Some(token)) = (info.get("port").and_then(Value::as_u64), info.get("token").and_then(Value::as_str)) else { return false };
    let Ok(stream) = TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port as u16)), Duration::from_secs(2)) else { return false };
    relay_over(stream, token, allow, std::io::stdin().lock(), std::io::stdout())
}

/// 中継の中身（テストでは、標準入出力の代わりを渡す）。
fn relay_over(stream: TcpStream, token: &str, allow: &[PathBuf], input: impl BufRead, output: impl Write + Send + 'static) -> bool {
    let Ok(mut writer) = stream.try_clone() else { return false };
    let mut reader = BufReader::new(stream);
    let allow: Vec<String> = allow.iter().map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()).display().to_string()).collect();
    if writeln!(writer, "{}", json!({ "token": token, "allow": allow })).is_err() {
        return false;
    }
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() || serde_json::from_str::<Value>(first.trim()).ok().and_then(|v| v.get("ok").and_then(Value::as_bool)) != Some(true) {
        return false;
    }
    eprintln!("PAKENOBA: connected to the open window.");
    // 画面からの答えを、標準出力へ。
    let mut output = output;
    let back = std::thread::spawn(move || {
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if writeln!(output, "{line}").and_then(|_| output.flush()).is_err() {
                break;
            }
        }
    });
    for line in input.lines() {
        let Ok(line) = line else { break };
        if writeln!(writer, "{line}").is_err() {
            break;
        }
    }
    // 標準入力が閉じたら、送った依頼の答えを受け取りきってから終わる。
    let _ = writer.shutdown(std::net::Shutdown::Write);
    let _ = back.join();
    true
}

impl super::App {
    /// 「MCP からこの画面を動かす」を入れたり切ったりする。
    pub(super) fn set_mcp_window(&mut self, ctx: &egui::Context, on: bool) {
        if on && self.bridge.is_none() {
            match Bridge::open(ctx.clone(), file()) {
                Ok(bridge) => self.bridge = Some(bridge),
                Err(e) => self.say(pakenoba_i18n::t!("Could not let MCP drive this window: {e}")),
            }
        } else if !on {
            self.bridge = None;
        }
    }

    /// 届いた依頼を、画面のスレッドで実行する（1 回の描き直しごと）。
    pub(super) fn run_bridge_jobs(&mut self) {
        let Some(bridge) = &self.bridge else { return };
        let jobs: Vec<Job> = bridge.jobs.try_iter().collect();
        let actors = bridge.actors.clone();
        for job in jobs {
            let mut actor = actors.lock().map(|a| a.get(&job.connection).cloned()).ok().flatten().unwrap_or_else(|| "mcp:client".to_owned());
            // その依頼のあいだだけ、つないだ側の `--allow` のフォルダも使ってよい。
            if job.message.get("method").and_then(Value::as_str) == Some("initialize") {
                self.track("ai_used", json!({ "feature": "mcp_window" }));
            }
            let kept = std::mem::replace(&mut self.allowed, job.folders.clone());
            let reply = super::mcp::handle(self, &mut actor, &job.message);
            self.allowed = kept;
            if let Ok(mut a) = actors.lock() {
                a.insert(job.connection, actor);
            }
            let _ = job.reply.send(reply);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 合言葉が違えば_断る() {
        let ctx = egui::Context::default();
        let bridge = Bridge::open(ctx, None).unwrap();
        let port = bridge.port();
        // 合言葉は、ファイルに書かないので、ここでは取り出せない。違う合言葉は断られる。
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        assert!(!relay_over(stream, "wrong", &[], "".as_bytes(), Vec::new()));
        drop(bridge);
    }

    #[test]
    fn 中継で_mcpの行が画面のアプリに届く() {
        let dir = std::env::temp_dir().join(format!("pakenoba-bridge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(FILE);
        let ctx = egui::Context::default();
        let mut app = super::super::App::new(&super::super::Initial::default());
        app.decals.clear();
        app.bridge = Some(Bridge::open(ctx, Some(file.clone())).unwrap());
        let info: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let (port, token) = (info["port"].as_u64().unwrap() as u16, info["token"].as_str().unwrap().to_owned());
        // 中継は別のスレッドで動かし、画面の側は、ここで依頼を処理する。
        let out = Arc::new(Mutex::new(Vec::<u8>::new()));
        struct Shared(Arc<Mutex<Vec<u8>>>);
        impl Write for Shared {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let shared = Shared(out.clone());
        let input = [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "vscode", "version": "1" } } }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "command_run", "arguments": { "name": "decal.add", "args": { "builtin": "logo" } } } }),
        ]
        .iter()
        .map(|l| format!("{l}\n"))
        .collect::<String>();
        let relay = std::thread::spawn(move || {
            let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            relay_over(stream, &token, &[], input.as_bytes(), shared)
        });
        let started = std::time::Instant::now();
        while !relay.is_finished() && started.elapsed() < Duration::from_secs(10) {
            app.run_bridge_jobs();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(relay.join().unwrap());
        let replies: Vec<Value> = String::from_utf8(out.lock().unwrap().clone()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert_eq!(replies[1]["result"]["isError"], false);
        // 画面のアプリに、柄が足され、記録には相手の名前が残る。
        assert_eq!(app.decals.len(), 1);
        assert!(app.log.iter().any(|e| e.actor == "mcp:vscode"));
        // 口を閉じると、ファイルも消える。
        app.bridge = None;
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
