#!/usr/bin/env python3
"""パケノバの見本パック（ZIP）を作る。

見本パックは、アプリ本体とは別に配る素材の詰め合わせ（GitHub のリリースに置き、アプリの素材の画面から取ってくる）。
中身は、架空のメーカーやブランドのロゴ（PNG）、箱の全面に使える四方連続の模様（SVG）、旧パケノバの見本（SVG）。
どれも画像生成 AI と、周期関数から作ったもので、実在の作品やブランドを元にしていない。

使い方:
    python3 tools/make_sample_pack.py --logos <ロゴの images フォルダ> --patterns <package_patterns_set06_wave_fields>
        --classic <旧パケノバの docs/reports/assets/RPT-0192> --out dist/pakenoba-samples-1.zip

必要なもの: Python 3.9 以上、Pillow。
"""

import argparse
import csv
import json
import re
import zipfile
from io import BytesIO
from pathlib import Path

from PIL import Image

PACK_VERSION = 1
# ロゴは、長いほうをこの画素数に縮める（40 mm で約 500 dpi）。
LOGO_PX = 800
# 模様の SVG の、1 周期（スーパータイル）の実寸。
PATTERN_MM = 120
KINDS = {"maker": "Maker", "brand": "Brand", "series": "Series", "grade": "Grade", "product": "Product"}


def logo_entries(folder: Path):
    for path in sorted(folder.glob("LOGO-*.png")):
        m = re.match(r"LOGO-(\d+)_([a-z]+)_(.+)\.png$", path.name)
        if not m:
            continue
        number, kind, name = m.groups()
        with Image.open(path) as im:
            im = im.convert("RGBA")
            # 透明な余白を切り落としてから縮める。
            box = im.getbbox()
            if box:
                im = im.crop(box)
            scale = LOGO_PX / max(im.size)
            if scale < 1:
                im = im.resize((round(im.width * scale), round(im.height * scale)), Image.LANCZOS)
            out = BytesIO()
            im.save(out, "PNG", optimize=True)
        width_mm = 40.0
        yield {
            "file": f"logos/{path.name}",
            "name": f"{name.replace('_', ' ')} ({KINDS.get(kind, kind)} {int(number)})",
            "kind": "logo",
            "size_mm": [width_mm, round(width_mm * im.height / im.width, 1)],
            "tags": ["logo", kind],
        }, out.getvalue()


def pattern_entries(folder: Path):
    rows = list(csv.DictReader(open(folder / "manifest.csv", encoding="utf-8-sig")))
    for row in rows:
        svg = (folder / row["svg_file"]).read_text(encoding="utf-8")
        # 実寸を書く（アプリは width と height の mm を、柄の大きさにする）。
        svg = re.sub(r'(<svg[^>]*?)\swidth="[^"]*"', r"\1", svg, count=1)
        svg = re.sub(r'(<svg[^>]*?)\sheight="[^"]*"', r"\1", svg, count=1)
        svg = svg.replace("<svg", f'<svg width="{PATTERN_MM}mm" height="{PATTERN_MM}mm"', 1)
        name = f"{row['pattern_name_ja']} / {row['palette_id']}"
        yield {
            "file": f"patterns/{Path(row['svg_file']).name}",
            "name": name,
            "kind": "pattern",
            "size_mm": [PATTERN_MM, PATTERN_MM],
            "tags": ["pattern", "seamless", row["field_style"], row["palette_id"]],
            "note": "Seamless in both directions; also symmetric under 90-degree rotation and mirroring (D4).",
        }, svg.encode("utf-8")


def classic_entries(folder: Path):
    for path in sorted(folder.glob("*.svg")):
        yield {
            "file": f"classic/{path.name}",
            "name": path.stem.replace("-", " "),
            "kind": "classic",
            "size_mm": None,
            "tags": ["classic"],
        }, path.read_bytes()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--logos", type=Path, required=True)
    ap.add_argument("--patterns", type=Path, required=True)
    ap.add_argument("--classic", type=Path)
    ap.add_argument("--license", default="CC0-1.0")
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    items = []
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.out, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        sources = [logo_entries(args.logos), pattern_entries(args.patterns)]
        if args.classic:
            sources.append(classic_entries(args.classic))
        for source in sources:
            for entry, data in source:
                z.writestr(entry["file"], data)
                items.append(entry)
        manifest = {
            "format": "pakenoba-sample-pack",
            "version": PACK_VERSION,
            "name": "PAKENOBA sample pack",
            "license": args.license,
            "note": "Made with image-generation AI and periodic functions by the PAKENOBA author. Fictional names; not based on existing works or brands.",
            "items": items,
        }
        z.writestr("pakenoba-samples.json", json.dumps(manifest, ensure_ascii=False, indent=1))
        z.writestr("README.txt", "PAKENOBA sample pack\n\nFictional logos (PNG), seamless wave-field patterns (SVG) and classic samples (SVG).\n"
                   f"License: {args.license}\nSee pakenoba-samples.json for the list.\n")
    print(f"{args.out}: {len(items)} items, {args.out.stat().st_size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
