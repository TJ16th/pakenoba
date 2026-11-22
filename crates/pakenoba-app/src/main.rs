//! パケノバ: パッケージデザイン専用の制作ソフト。
//!
//! 箱の形（型紙）を決め、立体の上で柄を置き、見せるための画像や型紙を書き出す。
//! 計算の中核は `pakenoba-core`。ここは、ウィンドウと描画。

mod app;
mod art;
mod gpu;
mod paint;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut initial = app::Initial::default();
    let mut i = 0;
    while i < args.len() {
        // 「--」で始まらないものは、開く案件のファイル（ファイルをアプリに関連付けて、ダブルクリックで開くときの渡し方）。
        if !args[i].starts_with("--") {
            initial.project = Some(std::path::PathBuf::from(&args[i]));
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
            ("--answer-replace", _) if i + 1 < args.len() => initial.answer_replace = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--psd", _) if i + 1 < args.len() => initial.psd = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--png", _) if i + 1 < args.len() => initial.png = Some(std::path::PathBuf::from(&args[i + 1])),
            ("--logo" | "--band", _) if i + 1 < args.len() => {
                // 「パネルの番号,横の位置,縦の位置,回転（度）」。位置はパネルの中での割合（0〜1）。
                let v: Vec<f32> = args[i + 1].split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() != 4 {
                    println!("{} は「パネルの番号,横の位置,縦の位置,回転（度）」で指定します。", args[i]);
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
                    println!("--views は、立体、表、裏を出すかどうかを、1 と 0 の 3 文字で指定します（例: 110）。");
                    return ExitCode::FAILURE;
                }
                initial.views = Some([on[0], on[1], on[2]]);
            }
            ("--no-guides", _) => {
                initial.guides = false;
                i += 1;
                continue;
            }
            _ => {
                println!("使い方: pakenoba [開く案件のファイル（.pknb）] [--fold 0〜1] [--yaw ラジアン] [--pitch ラジアン] [--thickness mm] [--open 0〜1] [--sides 型（0: 差し込み箱、2: ゲーブル型、100: ピロー型、そのほかの 3 以上: 角柱）] [--workspace 0〜2] [--bulge 度] [--width mm] [--height mm] [--no-guides] [--psd ファイル] [--png 柄にする画像] [--logo パネル,横,縦,回転] [--band パネル,横,縦,回転] [--select 柄の番号] [--views 立体・表・裏を出すか（例: 110）] [--back 裏へ移す柄の番号] [--assets 素材の欄を開く（1: 道具の右、2: 道具の左）]");
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
