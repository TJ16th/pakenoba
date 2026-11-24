//! 初回に取ってくるモデル（被写体を抽出の、高画質のモデル）。
//!
//! 大きいモデル（U²-Net、約 176 MB、Apache-2.0）は、アプリに入れず、使う人が「高画質」を選んだときに 1 回だけ取ってくる。
//! 取ってきたものは、使う人のデータのフォルダ（Windows なら `%LOCALAPPDATA%\pakenoba\models`）に置く。
//! 中身の印（SHA-256）が合わないものは使わない。絵を外へ送ることはない（モデルを取ってくるだけ）。

use pakenoba_i18n::t;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

/// 取ってくる場所（rembg の配布物。U²-Net の作者のモデルを ONNX にしたもの）。
const URL: &str = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2net.onnx";
const SHA256: &str = "8d10d2f3bb75ae3b6d527c77944fc5e7dcd94b29809d47a739a7a728a912b491";
const SIZE: u64 = 175_997_641;

/// 高画質のモデルの置き場所。
pub fn fine_model_path() -> Option<PathBuf> {
    crate::app::settings::data_dir().map(|dir| dir.join("models").join("u2net.onnx"))
}

/// 高画質のモデルが、もう取ってあるか。
pub fn fine_model_ready() -> bool {
    fine_model_path().is_some_and(|p| p.metadata().is_ok_and(|m| m.len() == SIZE))
}

/// 取ってくる様子。
#[derive(Clone, Debug, PartialEq)]
pub enum Fetch {
    Idle,
    /// 取ってきたバイト数と、全体。
    Running(u64, u64),
    Failed(String),
    Done,
}

static STATE: Mutex<Fetch> = Mutex::new(Fetch::Idle);
/// 取り終わった（か、失敗した）ことを、まだ画面に知らせていない。
static FINISHED: Mutex<Option<Result<(), String>>> = Mutex::new(None);

/// 取り終わったか失敗したかを、1 回だけ返す。
pub fn take_finished() -> Option<Result<(), String>> {
    FINISHED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

pub fn state() -> Fetch {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set(state: Fetch) {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = state;
    crate::effects::wake();
}

/// 取ってくるのを始める（もう始めていれば、何もしない）。
pub fn start() {
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, Fetch::Running(..)) {
            return;
        }
        *state = Fetch::Running(0, SIZE);
    }
    std::thread::spawn(|| {
        let result = fetch();
        *FINISHED.lock().unwrap_or_else(|e| e.into_inner()) = Some(result.clone());
        match result {
            Ok(()) => set(Fetch::Done),
            Err(why) => set(Fetch::Failed(why)),
        }
    });
}

fn fetch() -> Result<(), String> {
    let path = fine_model_path().ok_or_else(|| t!("There is no folder to keep the model in.").to_owned())?;
    let dir = path.parent().ok_or_else(|| t!("There is no folder to keep the model in.").to_owned())?;
    std::fs::create_dir_all(dir).map_err(|e| t!("Could not make the folder for the model: {e}"))?;
    let part = path.with_extension("onnx.part");
    let response = ureq::get(URL).call().map_err(|e| t!("Could not download the model: {e}"))?;
    let mut body = response.into_body();
    let mut reader = body.with_config().limit(SIZE + 1024).reader();
    let mut file = std::fs::File::create(&part).map_err(|e| t!("Could not save the model: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| t!("Could not download the model: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| t!("Could not save the model: {e}"))?;
        done += n as u64;
        if done % (4 << 20) < n as u64 {
            *STATE.lock().unwrap_or_else(|e| e.into_inner()) = Fetch::Running(done, SIZE);
            crate::effects::wake();
        }
    }
    file.sync_all().map_err(|e| t!("Could not save the model: {e}"))?;
    drop(file);
    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != SHA256 {
        let _ = std::fs::remove_file(&part);
        return Err(t!("The downloaded model was damaged, so it was not used. Please try again.").to_owned());
    }
    std::fs::rename(&part, &path).map_err(|e| t!("Could not save the model: {e}"))?;
    Ok(())
}
