# icons

This directory holds every platform icon the Tauri bundler embeds. They are
all generated from a single source: [`source.svg`](./source.svg).

## Regenerating

After editing `source.svg`, run from the project root:

```bash
pnpm tauri icon src-tauri/icons/source.svg
```

That regenerates every file under this directory in place, including:

| Target          | Files                                                                                          |
| --------------- | ---------------------------------------------------------------------------------------------- |
| Windows EXE     | `icon.ico` (embedded as a Win32 resource by `tauri-build`)                                     |
| Windows MSIX    | `Square*Logo.png`, `StoreLogo.png`                                                             |
| macOS           | `icon.icns`                                                                                    |
| Linux           | `32x32.png`, `64x64.png`, `128x128.png`, `128x128@2x.png`, `icon.png`                          |
| iOS / Android   | `ios/AppIcon-*.png`, `android/mipmap-*/ic_launcher*.png`                                       |

The set of files referenced by the Windows / macOS / Linux bundle is the
`bundle.icon` array in [`../tauri.conf.json`](../tauri.conf.json). The mobile
targets are picked up automatically by the `ios` / `android` subfolders.

## Using your own artwork

The simplest swap is to replace `source.svg` with a 1024×1024 squared design.
Any well-formed SVG works as long as it has no font dependencies; pure shapes
guarantee identical rasterisation on every host.

If you want to start from a PNG instead, pass it directly:

```bash
pnpm tauri icon path/to/your-icon.png
```

A 1024×1024 PNG with transparency is recommended.

## Brand notes

`source.svg` uses the same palette as the in-app TUI theme (see
`src/index.css`):

- background `#0b0d10`
- accent     `#00d3a7`
- border     `#2a3038`

The mark is a stylised lowercase `k` built from primitives only, so it reads
cleanly down to 16×16 (where most other glyph-based icons collapse into
illegible blobs).
