# 配布の手順

- 作成日: 2026-11-25
- 関連: `.github/workflows/release.yml`、`.github/workflows/ci.yml`、`tools/make_release.ps1`、`crates/pakenoba-app/src/app/updates.rs`

## はじめに 1 回だけ

1. GitHub に公開リポジトリ `TJ16th/pakenoba` を作り、押す（別の名前にするなら、`updates.rs` の `REPO` と README のリンクも変える）。
2. リポジトリの Settings → Secrets and variables → Actions に、`APTABASE_APP_KEY`（Aptabase の App Key。`A-EU-…` か `A-US-…`）を入れる。入れなければ、利用状況を送らないビルドになる。
3. Settings → Actions → General で、Workflow permissions が「Read and write」になっているか確かめる（Releases に置くため）。

## 版を出すたびに

1. `Cargo.toml` の `[workspace.package]` の `version` を上げる（例: `0.1.0` → `0.2.0`）。
2. `docs/release-notes.md` の「この版の変わったところ」を書く。
3. コミットして押す。CI（テストと clippy）が通るのを待つ。
4. 版のタグを付けて押す: `git tag v0.2.0` → `git push origin v0.2.0`。
5. Actions の「Release」が、テスト、ビルド、ZIP と `SHA256SUMS.txt` の作成、Releases への公開をする（20〜30 分）。

タグと `Cargo.toml` の版がそろっていないと、Release は止まる。

## 配布物の決まり

- ZIP の名前は `pakenoba-<版>-windows-x64-portable.zip`。中は 1 つのフォルダで、その直下に `pakenoba.exe`。アプリ内のアップデート（craft-update）は、この名前と `SHA256SUMS.txt` を Releases から探す。
- 版の名前（「パケノバについて」に出るもの）は `v<版>`。手元で `make_release.ps1`（`-Release` なし）で作ったものは `v<版> test<コミットの数>`。

## コード署名（あとで）

- 署名していない Windows の ZIP には、アップデートは「ダウンロードの場所」を出すだけ（その場での入れ替えは、署名したものだけ）。
- SignPath Foundation（オープンソース向けの無料の署名）に申し込む。条件は、ビルドが GitHub Actions で自動になっていること、二段階認証、過去のリリースが 1 回以上あること。「MIT または Apache-2.0」がデュアルライセンスの扱いにならないかは、申し込む前に確かめる。
- 署名の段階は、`release.yml` の「Build and package」と「Publish the release」の間に足す。

## 動画の形式

- 箱開けの動画は GIF だけで、H.264 は使っていない（方針メモの「H.264 の特許の扱い」は、MP4 を足すときに確かめる）。
