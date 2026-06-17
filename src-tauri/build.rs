//! Tauri build script.
//!
//! On Windows, `tauri-build` embeds `icons/icon.ico` as a resource for the
//! produced EXE. The repo ships a real `icon.ico` generated from
//! `icons/source.svg`, so the fallback below should never fire in normal use.
//! It only exists as a safety net for partial / broken checkouts so a fresh
//! clone can still run `cargo check` without an opaque linker error.
//!
//! To replace the brand mark:
//!   1. Edit `icons/source.svg`.
//!   2. From the project root, run `pnpm tauri icon src-tauri/icons/source.svg`.
//!
//! That overwrites every generated asset under `icons/` in place.

fn main() {
    #[cfg(target_os = "windows")]
    ensure_placeholder_icon();

    tauri_build::build()
}

#[cfg(target_os = "windows")]
fn ensure_placeholder_icon() {
    use std::path::Path;

    let icon = Path::new("icons/icon.ico");
    if icon.exists() {
        return;
    }
    if let Some(parent) = icon.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(icon, MINIMAL_ICO) {
        println!("cargo:warning=could not write placeholder icon.ico: {e}");
    } else {
        println!(
            "cargo:warning=icons/icon.ico was missing; wrote 1×1 placeholder. \
             Regenerate with `pnpm tauri icon src-tauri/icons/source.svg`."
        );
    }
}

/// Minimal valid ICO: single 1×1, 32-bpp BGRA, teal pixel + AND-mask byte.
///
/// Structure: ICONDIR(6) + ICONDIRENTRY(16) + BITMAPINFOHEADER(40) +
///            XOR pixel(4) + AND mask(4 padding) = 70 bytes total.
#[cfg(target_os = "windows")]
const MINIMAL_ICO: &[u8] = &[
    // ICONDIR
    0x00, 0x00, // reserved
    0x01, 0x00, // type = 1 (icon)
    0x01, 0x00, // count = 1
    // ICONDIRENTRY
    0x01, // width
    0x01, // height
    0x00, // colour count (0 = >256)
    0x00, // reserved
    0x01, 0x00, // colour planes
    0x20, 0x00, // bits per pixel = 32
    0x30, 0x00, 0x00, 0x00, // image data size = 48 bytes (BIH + pixel + mask)
    0x16, 0x00, 0x00, 0x00, // offset to image data = 22
    // BITMAPINFOHEADER (40 bytes)
    0x28, 0x00, 0x00, 0x00, // header size
    0x01, 0x00, 0x00, 0x00, // width = 1
    0x02, 0x00, 0x00, 0x00, // height = 2 (×2 for XOR+AND)
    0x01, 0x00, // planes = 1
    0x20, 0x00, // bpp = 32
    0x00, 0x00, 0x00, 0x00, // compression = BI_RGB
    0x00, 0x00, 0x00, 0x00, // image size (0 ok for BI_RGB)
    0x00, 0x00, 0x00, 0x00, // x ppm
    0x00, 0x00, 0x00, 0x00, // y ppm
    0x00, 0x00, 0x00, 0x00, // colours used
    0x00, 0x00, 0x00, 0x00, // important colours
    // XOR image: 1 pixel BGRA (teal accent)
    0xA7, 0xD3, 0x00, 0xFF,
    // AND mask: 4 bytes (row padded to dword), 0 = opaque
    0x00, 0x00, 0x00, 0x00,
];
