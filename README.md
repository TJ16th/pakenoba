# PAKENOBA

**English** | [日本語](README.ja.md)

PAKENOBA is an app made for package design. Choose a box style, place artwork on the 3D box, and export presentation images and print-ready PDFs, all in one app. It is free, and the source code is open.

<!-- Promo video (English, with translated lyrics): edit this file on GitHub and drag pakenoba_promo_en_lyrics_readme.mp4 below this line -->

![The Design step](docs/images/app-design.jpg)

## Download

Get `pakenoba-<version>-windows-x64-portable.zip` from [Releases](https://github.com/TJ16th/pakenoba/releases), unzip it anywhere, and open `pakenoba.exe`. No installation is needed.

- For now, only Windows (64-bit) is supported.
- Windows may show "Windows protected your PC" when you start it, because the app is not code-signed yet. Click "More info" → "Run anyway". We plan to apply for free code signing for open-source projects (SignPath Foundation).
- When a new version is out, PAKENOBA tells you at start (or via Help → Check for updates…). For now it shows where to download it; once the app is signed, it will be able to update itself in place.

The interface is in Japanese and English.

## What it can do

- **Box styles**: tuck-end box, gable box, pillow box (with and without a handle), and 3- to 8-sided prisms. You can also make a folding carton from its **ECMA** number (130 styles of the A group, such as A20.20.03.01) and compare the styles by their pictures, or import a dieline from **DXF**. When you change the dimensions, the dieline and the 3D box update right away. Dimensions you use often can be saved in "Your box types".
- **Placing artwork**: images (PNG, JPEG, PSD, SVG), text and shapes, on the 3D box or on the dieline. Artwork that spans panels continues across folds and seams. You can also place artwork on the back of the board (the inside of the box).
- **Layers**: blend modes, opacity, masks, groups, adjustments (levels, hue/saturation and more), select subject, make the background transparent, flip, and vectorize (VectorCraft Image Trace).
- **Board and finishes**: preview the board and coating (varnish, PP lamination), foil stamping, embossing and debossing, spot UV and white ink in 3D.
- **Export**:
  - A layered PSD of the dieline, with bleed
  - A **print-ready PDF** for the printer (actual size; cut lines, creases and finish plates as spot colors; the artwork can be converted to **CMYK**)
  - An actual-size dieline split into A4 or A3 pages
  - 16:9 images, an animation of the box opening (GIF), 3D models (GLB, OBJ), and images to hand to generative AI
- **Working with AI**: drive PAKENOBA with its built-in agent (an Anthropic or OpenAI API key), or over **MCP** from VS Code or Claude Desktop. Turn on Edit → Preferences → "Let MCP drive this window" and the open window changes as the AI works. Sample settings are in the `mcp` folder of the download.
- **Organizing assets**: register folders of assets, browse them as large pictures, and search by name or tag. A local AI (Ollama) can name and tag them for you.

The dielines PAKENOBA makes are for checking designs. They are not official data that can be relied on for manufacturing, cutting dies, die-cutting or print registration.

## Usage data

The first time you start PAKENOBA, it asks whether it may send anonymous usage data. It only sends that the app started, the box styles you chose, the kinds of files you exported, and which AI features you used. It never sends the contents of your projects, file names, pictures or text. You can turn it off at any time, and see exactly what was sent, in Edit → Preferences → Usage data. See [docs/usage-data.md](docs/usage-data.md) for details.

Only the official builds published on Releases send anything. If you build the app yourself from the source code, it has no key to send with, so it sends nothing and never asks.

## Building from source

You need Rust (1.95 or later).

```
cargo run --release -p pakenoba-app
```

Run the tests with `cargo test --workspace`.

| Path | Contents |
|---|---|
| `crates/pakenoba-core` | The core: dielines, folding, how panels connect, dieline checks, ECMA and DXF, project files |
| `crates/pakenoba-app` | The app: window, 3D view, painting artwork, export, agent and MCP |
| `crates/pakenoba-i18n` | Interface translations (add a table to `catalogs/` to add a language) |
| `docs/` | Design documents (in Japanese) |
| `spikes/` | Prototypes used to check the design |
| `tools/` | Tools for making the download (`make_release.ps1`) |

How to publish a release is in [docs/releasing.md](docs/releasing.md).

## License

The code is licensed under MIT or Apache-2.0, at your choice ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

The names "パケノバ" and "PAKENOBA" and the logo are not covered by the code license. If you distribute a modified version, please use a different name and logo. See [TRADEMARKS.md](TRADEMARKS.md) for details.

Main components and models used:

- [craft-libs](https://github.com/storytold/craft-libs) (PSD, SVG, color management, updates) and [VectorCraft](https://github.com/storytold/vectorcraft) Image Trace (MIT or Apache-2.0)
- Face and eye detection for the peek view: MediaPipe BlazeFace (Apache-2.0; `crates/pakenoba-app/assets/blazeface-LICENSE.txt`)
- Select subject: U²-Net (Apache-2.0; the small u2netp model is bundled, and the larger model is downloaded once only when you choose "High quality")
- The list of ECMA style numbers and how closures are grouped follow [pacxio](https://pypi.org/project/pacxio/) (MIT); the shapes are drawn by PAKENOBA itself
- The built-in CMYK profile: made by craft-cms to match the FOGRA39 measurements (CC0)

Image processing runs on your PC and never sends your pictures anywhere. The only things sent out are what you ask the agent or a generative AI to do.
