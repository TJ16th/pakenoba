# パケノバ（PAKENOBA）

パッケージデザイン専用の制作ソフトです。箱の型を決め、立体の上で柄を置き、見せるための画像や、印刷所に渡す PDF を書き出すところまでを 1 本で行います。無料で、ソースコードも公開しています。

*PAKENOBA is a free, open-source app for designing packages: choose a box style, place artwork on the 3D box, and export images and print-ready PDFs. The interface is in Japanese and English.*

![デザインの工程](docs/images/app-design.jpg)

## ダウンロード

[Releases](https://github.com/TJ16th/pakenoba/releases) から `pakenoba-<版>-windows-x64-portable.zip` を取ってきて、好きな場所に解凍し、`pakenoba.exe` を開きます。インストールは要りません。

- いまは Windows（64 ビット）用だけです。
- 起動すると「Windows によって PC が保護されました」と出ることがあります。まだコード署名をしていないためです。「詳細情報」→「実行」で開けます。署名は、オープンソース向けの無料の署名（SignPath Foundation）に申し込む予定です。
- 新しい版が出ると、起動したときに知らせます（「ヘルプ」→「アップデートを確かめる」でも）。いまはダウンロードの場所を出します。署名したあとは、その場で入れ替えられるようになります。

## できること

- **箱の型**: 差し込み箱、ゲーブル型、ピロー型（持ち手つきと、持ち手なし）、三角柱〜八角柱。紙器の型の番号 **ECMA**（A 類の 130 型。A20.20.03.01 など）から作ることも、**DXF** の型を読み込むこともできます。寸法を変えると、型紙と立体がその場で変わります。よく使う寸法は「自分の型」に残せます。
- **柄を置く**: 画像（PNG、JPEG、PSD、SVG）、文字、図形を、立体の上でも展開図の上でも置けます。面をまたぐ柄は、折り目や合わせ目を越えて続きます。紙の裏（箱の内側）にも置けます。
- **レイヤー**: 描画モード、不透明度、マスク、グループ、調整（レベル補正、色相・彩度など）、被写体を選択、背景を透明に、ベクター化（VectorCraft の画像トレース）。
- **紙と加工**: 紙とコート（ニス、PP）の見え方と、箔押し、エンボス・デボス、部分 UV、白版を、立体で確かめられます。
- **書き出し**:
  - 型紙の PSD（レイヤーつき、塗り足しつき）
  - 印刷所に渡す **入稿用 PDF**（実寸。切り線、折り線、加工の版を特色で入れる。絵を **CMYK** にもできる）
  - 実寸の型紙を A4 や A3 に分けた PDF
  - 16:9 の画像、箱が開く動画（GIF）、3D モデル（GLB、OBJ）、生成 AI に渡す画像
- **AI と一緒に使う**: パケノバの中のエージェント（Anthropic か OpenAI の API キー）か、**MCP** で VS Code や Claude Desktop から動かせます。「編集」→「環境設定」→「MCP からこの画面を動かす」を入れると、開いている画面がその場で動きます。設定の見本は、配布物の `mcp` フォルダにあります。
- **素材の整理**: 素材のフォルダを登録して、大きな絵で見たり、名前やタグで探したりできます。手元の AI（Ollama）で、名前とタグを付けられます。

このソフトが出力する型紙は、デザインを確かめるためのものです。製造、抜き型、裁断、印刷位置の保証に使える正式なデータではありません。

## 利用状況の送信

はじめて起動したときに、匿名の利用状況を送ってよいかを聞きます。送るのは、起動したこと、選んだ箱の型、書き出したファイルの種類、使った AI の機能だけです。案件の中身、ファイルの名前、絵、文字は送りません。「編集」→「環境設定」→「利用状況」で、いつでも切れ、送ったものをそのまま見られます。詳しくは [docs/usage-data.md](docs/usage-data.md) を見てください。

自分でビルドしたものは、何も送りません。

## ソースからビルドする

Rust（1.95 以降）が要ります。

```
cargo run --release -p pakenoba-app
```

テストは `cargo test --workspace` で動きます。

| 場所 | 内容 |
|---|---|
| `crates/pakenoba-core` | 中核。型紙、折りの計算、面のつながり、型紙の検査、ECMA と DXF、案件のファイル |
| `crates/pakenoba-app` | アプリ本体。ウィンドウ、立体の表示、柄の描き込み、書き出し、エージェントと MCP |
| `crates/pakenoba-i18n` | 画面の文字の訳（`catalogs/` に表を足すと、言語を増やせる） |
| `docs/` | 設計の文書 |
| `spikes/` | 設計を確かめたときの試作 |
| `tools/` | 配布物を作る道具（`make_release.ps1`） |

配布の手順は [docs/releasing.md](docs/releasing.md) にあります。

## ライセンス

コードは「MIT または Apache-2.0」です。どちらかを選んで使えます（[LICENSE-MIT](LICENSE-MIT)、[LICENSE-APACHE](LICENSE-APACHE)）。

「パケノバ」「PAKENOBA」の名称とロゴは、コードのライセンスの対象外です。改造したものを配るときは、別の名前とロゴにしてください。詳しくは [TRADEMARKS.md](TRADEMARKS.md) を見てください。

使っている主な部品とモデル:

- [craft-libs](https://github.com/storytold/craft-libs)（PSD、SVG、色の管理、アップデート）と、[VectorCraft](https://github.com/storytold/vectorcraft) の画像トレース（MIT または Apache-2.0）
- のぞき込みの顔と目の検出: MediaPipe BlazeFace（Apache-2.0。`crates/pakenoba-app/assets/blazeface-LICENSE.txt`）
- 被写体を選択: U²-Net（Apache-2.0。軽いモデル u2netp を同梱し、「高画質」を選んだときだけ大きいモデルを 1 回取ってくる）
- ECMA の型の番号の一覧と、閉じ方の分け方: [pacxio](https://pypi.org/project/pacxio/)（MIT）に合わせた。形はパケノバで描き起こしている
- 内蔵の CMYK プロファイル: craft-cms が FOGRA39 の測定値に合わせて作ったもの（CC0）

画像の処理は PC の中で動き、絵を外へ送りません。外へ送るのは、エージェントや生成 AI に頼んだときに、頼んだものだけです。
