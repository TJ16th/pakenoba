<#
.SYNOPSIS
  パケノバを、知り合いに渡せる ZIP にする（Windows 用）。

.DESCRIPTION
  1. リリース用にビルドする（cargo build --release -p pakenoba-app）。
  2. dist\pakenoba-<日付>-<コミット>-windows-x64\ に、pakenoba.exe、はじめに.txt、ライセンスの条文を集める。
  3. そのフォルダを ZIP にする（dist\pakenoba-<日付>-<コミット>-windows-x64.zip）。
  見本パック（dist\pakenoba-samples-1.zip）があれば、-WithSamples で ZIP の横に写しを置く（本体の ZIP には入れない。大きいので別に渡す）。

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\make_release.ps1
  powershell -ExecutionPolicy Bypass -File tools\make_release.ps1 -SkipBuild
#>
param(
    # ビルドを飛ばして、いまある target\release\pakenoba.exe を使う。
    [switch]$SkipBuild,
    # 見本パックの写しも、渡すフォルダに置く。
    [switch]$WithSamples
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# 名前に入れる日付とコミット（git がなければ日付だけ）。
$date = Get-Date -Format 'yyyyMMdd'
$commit = ''
try { $commit = (git rev-parse --short HEAD 2>$null).Trim() } catch { }
$dirty = ''
try { if ((git status --porcelain -- crates 2>$null)) { $dirty = '-dirty' } } catch { }
$name = if ($commit) { "pakenoba-$date-$commit$dirty-windows-x64" } else { "pakenoba-$date-windows-x64" }

if (-not $SkipBuild) {
    Write-Host 'リリース用にビルドしています（初回は数分かかります）…'
    cargo build --release -p pakenoba-app
    if ($LASTEXITCODE -ne 0) { throw 'ビルドに失敗しました。' }
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
$readme = $readme.Replace('{VERSION}', $name)
[System.IO.File]::WriteAllText((Join-Path $out 'はじめに.txt'), $readme, (New-Object System.Text.UTF8Encoding $true))
New-Item -ItemType Directory -Path (Join-Path $out 'licenses') | Out-Null
foreach ($f in 'LICENSE-MIT', 'LICENSE-APACHE', 'TRADEMARKS.md') {
    if (Test-Path (Join-Path $root $f)) { Copy-Item (Join-Path $root $f) (Join-Path $out "licenses\$f") }
}
$u2 = Join-Path $root 'crates\pakenoba-app\assets\u2netp-LICENSE.txt'
if (Test-Path $u2) { Copy-Item $u2 (Join-Path $out 'licenses\u2netp-LICENSE.txt') }

$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $out -DestinationPath $zip -CompressionLevel Optimal

if ($WithSamples) {
    $samples = Join-Path $dist 'pakenoba-samples-1.zip'
    if (Test-Path $samples) { Write-Host "見本パック: $samples （本体とは別に渡してください）" } else { Write-Warning '見本パック（dist\pakenoba-samples-1.zip）がありません。' }
}

$size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host ''
Write-Host "できました: $zip （$size MB）"
Write-Host 'Google Drive などの共有リンクで渡すのがおすすめです（メールの添付は exe 入りの ZIP がはじかれやすい）。'
