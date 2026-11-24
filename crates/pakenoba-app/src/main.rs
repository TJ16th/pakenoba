//! パケノバ: パッケージデザイン専用の制作ソフト。
//!
//! 箱の形（型紙）を決め、立体の上で柄を置き、見せるための画像や型紙を書き出す。
//! 計算の中核は `pakenoba-core`。ここは、ウィンドウと描画。

// Windows のリリース用のビルドでは、ダブルクリックで起動したときに、後ろにコンソールの窓を開かない。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;

/// 画面に出す版の名前。配布用のビルド（tools/make_release.ps1）では「v0.0.0 test42」のように、
/// 環境変数 PAKENOBA_BUILD で付ける。ふだんのビルドでは「v0.0.0 dev」。
pub(crate) const VERSION_LABEL: &str = match option_env!("PAKENOBA_BUILD") {
    Some(label) => label,
    None => concat!("v", env!("CARGO_PKG_VERSION"), " dev"),
};
mod art;
mod blend;
mod effects;
mod env;
mod models;
mod gpu;
mod paint;
mod print;
mod text;
mod vector;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `pakenoba mcp …`: 画面を出さずに、MCP のサーバーとして動く。
    if args.first().map(String::as_str) == Some("mcp") {
        return app::mcp::serve(&args[1..]);
    }
    // 案件のファイルも、確かめるための引数もなく起動したら、はじめの画面（ホーム）を出す。
    let mut initial = app::Initial { home: args.iter().all(|a| a == "--no-splash" || a == "--debug"), ..Default::default() };
    let mut i = 0;
    while i < args.len() {
        // 「--」で始まらないものは、開く案件のファイル（ファイルをアプリに関連付けて、ダブルクリックで開くときの渡し方）。
        if !args[i].starts_with("--") {
            initial.project.push(std::path::PathBuf::from(&args[i]));
            i += 1;
            continue;
        }
        let value = args.get(i + 1).and_then(|v| v.parse::<f32>().ok());
        match (args[i].as_str(), value) {
            ("--fold", Some(v)) => initial.fold = v,
            ("--yaw", Some(v)) => initial.yaw = v,
            ("--pitch", Some(v)) => initial.pitch = v,
            ("--sides", Some(v)) => initial.sides = v as usize,
            ("--workspace", Some(v)) => initial.workspace = v as usize,
            ("--open", Some(v)) => initial.open = v,
            ("--open-bottom", Some(v)) => initial.open_bottom = v,
            ("--bulge", Some(v)) => initial.bulge = Some(v.clamp(0.0, 90.0)),
            ("--width", Some(v)) => initial.width = Some(v.clamp(40.0, 300.0)),
            ("--height", Some(v)) => initial.height = Some(v.clamp(40.0, 300.0)),
            ("--thickness", Some(v)) => initial.thickness = v.clamp(0.3, 6.0),
            ("--answer-open", _) if i + 1 < args.len() => initial.answer_open = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--answer-save", _) if i + 1 < args.len() => initial.answer_save = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--recovery-dir", _) if i + 1 < args.len() => initial.recovery_dir = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--backup-seconds", Some(v)) => initial.backup_seconds = v as f64,
            ("--answer-export", _) if i + 1 < args.len() => initial.answer_export = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--settings", _) if i + 1 < args.len() => initial.settings = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--answer-folder", _) if i + 1 < args.len() => initial.answer_folder = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--library", _) => {
                initial.library = true;
                i += 1;
                continue;
            }
            ("--answer-replace", _) if i + 1 < args.len() => initial.answer_replace = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--psd", _) if i + 1 < args.len() => initial.psd = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--png", _) if i + 1 < args.len() => initial.png = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--logo" | "--band", _) if i + 1 < args.len() => {
                // 「パネルの番号,横の位置,縦の位置,回転（度）」。位置はパネルの中での割合（0〜1）。
                let v: Vec<f32> = args[i + 1].split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() != 4 {
                    println!("{} takes \"panel number,horizontal position,vertical position,rotation (degrees)\".", args[i]);
                    return ExitCode::FAILURE;
                }
                let at = Some([v[0], v[1], v[2], v[3]]);
                if args[i] == "--logo" { initial.logo = at } else { initial.band = at }
            }
            ("--select", Some(v)) => initial.select = Some(v as usize),
            ("--back", Some(v)) => initial.back.push(v as usize),
            ("--assets", Some(v)) => initial.assets = Some(v < 1.5),
            ("--debug", _) => {
                initial.debug = true;
                i += 1;
                continue;
            }
            ("--views", _) if i + 1 < args.len() => {
                // 「立体、表、裏」を出すかどうかを、1 と 0 の 3 文字で（例: 110）。
                let on: Vec<bool> = args[i + 1].chars().map(|c| c == '1').collect();
                if on.len() != 3 || !on.contains(&true) {
                    println!("--views takes three digits, 1 or 0, for whether to show the 3D view, the front and the back (for example 110).");
                    return ExitCode::FAILURE;
                }
                initial.views = Some([on[0], on[1], on[2]]);
            }
            // ガイドは、はじめは隠す（2026-11-23）。--no-guides は、前の使い方のために残す。
            ("--peek", _) if i + 1 < args.len() => {
                // のぞき込みを、決めた目の位置（mm、画面の中央から。x 右、y 上、z 手前）で見せる。カメラは使わない。
                let parts: Vec<f32> = args[i + 1].split(',').filter_map(|v| v.trim().parse().ok()).collect();
                let [x, y, z] = parts[..] else {
                    println!("--peek takes x,y,z in mm (for example 200,0,500).");
                    return ExitCode::FAILURE;
                };
                initial.peek = Some([x, y, z]);
                initial.workspace = 2;
            }
            ("--no-splash", _) => {
                initial.splash = false;
                i += 1;
                continue;
            }
            ("--guides", _) => {
                initial.guides = true;
                i += 1;
                continue;
            }
            ("--no-guides", _) => {
                initial.guides = false;
                i += 1;
                continue;
            }
            _ => {
                println!("Usage: pakenoba [project file (.pknb)] [--fold 0-1] [--yaw radians] [--pitch radians] [--thickness mm] [--open 0-1] [--sides shape (0: tuck-end box, 2: gable box, 100: pillow box with handle, 101: pillow box, any other 3 or more: prism)] [--workspace 0-2] [--bulge degrees] [--width mm] [--height mm] [--guides] [--no-splash] [--psd file] [--png image to place] [--logo panel,x,y,rotation] [--band panel,x,y,rotation] [--select artwork number] [--views show 3D, front, back (for example 110)] [--back artwork number to move to the back] [--peek x,y,z eye position in mm for the peek-in view, without a camera] [--assets open the assets panel (1: right of the tools, 2: left of the tools)] [--library open the Assets screen]");
                return if args[i] == "--help" { ExitCode::SUCCESS } else { ExitCode::FAILURE };
            }
        }
        i += 2;
    }
    match app::run(initial) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Could not open the window: {e}");
            ExitCode::FAILURE
        }
    }
}
