//! Generates `trayTemplate.png` (menu bar) in OUT_DIR for `include_bytes!`.
//!
//! If `../dist` is missing (frontend not built yet), creates a minimal stub so
//! `cargo check` / `cargo build` on the Rust workspace succeed without `npm run build`.

use std::env;
use std::fs::{self, File};
use std::path::PathBuf;

fn main() {
    ensure_frontend_dist_stub();
    ensure_app_icon_png();
    let dist_index =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../dist/index.html");
    println!("cargo:rerun-if-changed={}", dist_index.display());

    tauri_build::build();

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let path = out_dir.join("trayTemplate.png");
    write_ring_rgba_png(&path, 22, 22, 8.0, 5.0).expect("write trayTemplate.png");

    println!("cargo:rerun-if-changed=build.rs");
}

fn ensure_frontend_dist_stub() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest_dir.join("../dist");
    let index = dist.join("index.html");
    if index.exists() {
        return;
    }
    let _ = fs::create_dir_all(&dist);
    let stub = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"/><title>GitPass</title></head>
<body><p>Run <code>npm run build</code> in <code>gitpass-desktop/</code> for the full UI.</p></body>
</html>
"#;
    if std::fs::write(&index, stub).is_ok() {
        eprintln!(
            "gitpass-desktop: created stub {} (run npm run build in gitpass-desktop/ for production assets)",
            index.display()
        );
    }
}

/// Default app window icon (512×512) when `icons/icon.png` is not committed yet.
fn ensure_app_icon_png() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon_path = manifest_dir.join("icons/icon.png");
    if icon_path.exists() {
        println!("cargo:rerun-if-changed={}", icon_path.display());
        return;
    }
    if let Some(parent) = icon_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // Dark panel + white ring (matches tray motif); replace via `tauri icon` when ready.
    write_ring_rgba_png(&icon_path, 512, 512, 180.0, 120.0).expect("write icons/icon.png");
    eprintln!(
        "gitpass-desktop: created placeholder {} — run `npm run tauri icon <path.png>` for branded assets",
        icon_path.display()
    );
}

fn write_ring_rgba_png(
    path: &std::path::Path,
    w: u32,
    h: u32,
    r_outer: f32,
    r_inner: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let cx = (w - 1) as f32 / 2.0;
    let cy = (h - 1) as f32 / 2.0;

    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let idx = ((y * w + x) * 4) as usize;
            if w >= 64 {
                // App icon: solid dark background
                rgba[idx] = 26;
                rgba[idx + 1] = 26;
                rgba[idx + 2] = 46;
                rgba[idx + 3] = 255;
            }
            if d <= r_outer && d >= r_inner {
                rgba[idx] = 255;
                rgba[idx + 1] = 255;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            }
        }
    }

    let f = File::create(path)?;
    let mut enc = png::Encoder::new(f, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    writer.write_image_data(&rgba)?;
    writer.finish()?;
    Ok(())
}
