<#
.SYNOPSIS
  パケノバを、知り合いに渡せる ZIP にする（Windows 用）。

.DESCRIPTION
  1. リリース用にビルドする（cargo build --release -p pakenoba-app）。
  2. dist\pakenoba-v<版>-test<通し番号>-windows-x64\ に、pakenoba.exe、はじめに.txt、ライセンスの条文、
     MCP の設定の見本（mcp\）を集める。
  3. そのフォルダを ZIP にする（dist\pakenoba-v<版>-test<通し番号>-windows-x64.zip）。
  版の名前は「v0.0.0 test42」の形で自動で付ける（0.0.0 は Cargo.toml の version、42 はコミットの数）。
  -SkipBuild のときは、前にビルドした exe の中の版の名前のまま（ZIP の名前だけ新しくなる）。
  見本パック（dist\pakenoba-samples-1.zip）があれば、-WithSamples で ZIP の横に写しを置く（本体の ZIP には入れない。大きいので別に渡す）。

  -Release のときは、公開用の名前にする（版の名前は「v0.1.0」、ZIP は dist\pakenoba-0.1.0-windows-x64-portable.zip）。
  アプリ内のアップデート（craft-update）は、この名前と、横に作る SHA256SUMS.txt を GitHub の Releases から探す。
  GitHub Actions の .github/workflows/release.yml が、版のタグを付けたときに、これで作って Releases に置く。

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\make_release.ps1
  powershell -ExecutionPolicy Bypass -File tools\make_release.ps1 -SkipBuild
#>
param(
    # ビルドを飛ばして、いまある target\release\pakenoba.exe を使う。
    [switch]$SkipBuild,
    # 見本パックの写しも、渡すフォルダに置く。
    [switch]$WithSamples,
    # 公開用の名前にする（テストの通し番号を付けない）。
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# 版の名前: Cargo.toml の version と、コミットの数から付ける通し番号（例: v0.0.0 test42）。
# コミットが増えるたびに番号が上がるので、渡した版を区別できる。
$version = '0.0.0'
$toml = Get-Content (Join-Path $root 'Cargo.toml') -Raw -Encoding UTF8
if ($toml -match '(?m)^version\s*=\s*"([^"]+)"') { $version = $Matches[1] }
$count = ''
try { $count = (git rev-list --count HEAD 2>$null).Trim() } catch { }
if (-not $count) { $count = Get-Date -Format 'yyyyMMddHHmm' }
$commit = ''
try { $commit = (git rev-parse --short HEAD 2>$null).Trim() } catch { }
$dirty = ''
try { if ((git status --porcelain -- crates 2>$null)) { $dirty = '+' } } catch { }
$label = "v$version test$count$dirty"
$date = Get-Date -Format 'yyyy-MM-dd'
$name = "pakenoba-v$version-test$count-windows-x64"
if ($Release) {
    $label = "v$version"
    $name = "pakenoba-$version-windows-x64-portable"
}
Write-Host "版: $label（$date、コミット $commit）"

if (-not $SkipBuild) {
    Write-Host 'リリース用にビルドしています（初回は数分かかります）…'
    # アプリの「パケノバについて」と起動画面に、この版の名前を出す。
    $env:PAKENOBA_BUILD = $label
    cargo build --release -p pakenoba-app
    $code = $LASTEXITCODE
    Remove-Item Env:PAKENOBA_BUILD
    if ($code -ne 0) { throw 'ビルドに失敗しました。' }
}

$exe = Join-Path $root 'target\release\pakenoba.exe'
if (-not (Test-Path $exe)) { throw "見つかりません: $exe" }

$dist = Join-Path $root 'dist'
$out = Join-Path $dist $name
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Path $out | Out-Null

Copy-Item $exe (Join-Path $out 'pakenoba.exe')
# はじめに.txt は UTF-8（BOM つき。古いメモ帳でも文字化けしないように）。日付とコミットを差し込む。
$readme = Get-Content (Join-Path $PSScriptRoot 'release\はじめに.txt') -Raw -Encoding UTF8
$readme = $readme.Replace('{VERSION}', "$label（$date、コミット $commit）")
[System.IO.File]::WriteAllText((Join-Path $out 'はじめに.txt'), $readme, (New-Object System.Text.UTF8Encoding $true))
New-Item -ItemType Directory -Path (Join-Path $out 'licenses') | Out-Null
foreach ($f in 'LICENSE-MIT', 'LICENSE-APACHE', 'TRADEMARKS.md') {
    if (Test-Path (Join-Path $root $f)) { Copy-Item (Join-Path $root $f) (Join-Path $out "licenses\$f") }
}
$u2 = Join-Path $root 'crates\pakenoba-app\assets\u2netp-LICENSE.txt'
if (Test-Path $u2) { Copy-Item $u2 (Join-Path $out 'licenses\u2netp-LICENSE.txt') }

# AI のアプリ（VS Code、Claude Desktop）から使うときの、MCP の設定の見本と説明。
$mcp = Join-Path $PSScriptRoot 'release\mcp'
if (Test-Path $mcp) { Copy-Item $mcp (Join-Path $out 'mcp') -Recurse }

$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $out -DestinationPath $zip -CompressionLevel Optimal

if ($WithSamples) {
    $samples = Join-Path $dist 'pakenoba-samples-1.zip'
    if (Test-Path $samples) { Write-Host "見本パック: $samples （本体とは別に渡してください）" } else { Write-Warning '見本パック（dist\pakenoba-samples-1.zip）がありません。' }
}

if ($Release) {
    # アップデートの確かめに使う、ファイルの SHA-256（GNU の sha256sum と同じ形: 「値  名前」）。
    $hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
    [System.IO.File]::WriteAllText((Join-Path $dist 'SHA256SUMS.txt'), "$hash  $name.zip`n", (New-Object System.Text.UTF8Encoding $false))
}

$size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host ''
Write-Host "できました: $zip （$size MB）"
Write-Host 'Google Drive などの共有リンクで渡すのがおすすめです（メールの添付は exe 入りの ZIP がはじかれやすい）。'
