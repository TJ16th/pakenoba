//! 新パケノバ 試作 S01: 土台の確認。
//!
//! 確かめること:
//! 1. craft-libs の部品を、外部のアプリから特定コミット指定で使えるか
//! 2. PSD を読んで箱の面に貼れるか
//! 3. 日本語を入力できるか（特に Windows）
//! 4. 外部ソフトなしで動画を書き出せるか

mod app;
mod headless;
mod psd_io;
mod raster;
mod scene;
mod video;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--headless") {
        let out_dir = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "out".into()));
        let psd = args.get(2).map(PathBuf::from);
        return match headless::run(&out_dir, psd.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("失敗: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.first().map(String::as_str) == Some("--help") {
        println!("使い方:\n  pakenova-spike-s01                            ウィンドウを開く\n  pakenova-spike-s01 --headless [出力先] [PSD]  画面なしで PSD、静止画、動画を確認する");
        return ExitCode::SUCCESS;
    }
    match app::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ウィンドウを開けませんでした: {e}");
            ExitCode::FAILURE
        }
    }
}
