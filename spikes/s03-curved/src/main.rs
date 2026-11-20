//! 新パケノバ 試作 S03: 曲面を含む型（手提げ型）。
//!
//! 確かめること:
//! 1. 型紙（パネルと折り線）を正本にして、展開図と立体の両方を出せるか
//! 2. 紙の厚みを、単純なルールで折りに反映できるか
//! 3. 開き具合を連続的に変えられるか
//! 4. wgpu で描けるか

mod app;
mod art;
mod gpu;
mod math;
mod template;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut initial = app::Initial::default();
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).and_then(|v| v.parse::<f32>().ok());
        match (args[i].as_str(), value) {
            ("--fold", Some(v)) => initial.fold = v,
            ("--yaw", Some(v)) => initial.yaw = v,
            ("--pitch", Some(v)) => initial.pitch = v,
            ("--sides", Some(v)) => initial.sides = v as usize,
            ("--bulge", Some(v)) => initial.bulge = v.clamp(0.0, 75.0),
            ("--thickness", Some(v)) => initial.thickness = v.clamp(0.3, 6.0),
            ("--psd", _) if i + 1 < args.len() => initial.psd = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--no-guides", _) => {
                initial.guides = false;
                i += 1;
                continue;
            }
            _ => {
                println!("使い方: pakenova-spike-s03 [--fold 0〜1] [--yaw ラジアン] [--pitch ラジアン] [--thickness mm] [--sides 型（1: 手提げ型、2: 差し込み箱、3 以上: 角柱）] [--bulge 度] [--no-guides] [--psd ファイル]");
                return if args[i] == "--help" { ExitCode::SUCCESS } else { ExitCode::FAILURE };
            }
        }
        i += 2;
    }
    match app::run(initial) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ウィンドウを開けませんでした: {e}");
            ExitCode::FAILURE
        }
    }
}
