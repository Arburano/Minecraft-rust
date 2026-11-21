//! Writes all fallback textures as one BMP sheet (8x scale, 4 per row) so they
//! can be looked at without the game: `cargo run -p assets --example
//! fallback_preview -- <out.bmp>`.

use std::process::ExitCode;

use assets::{FALLBACK_KEYS, fallback_texture};

const SCALE: u32 = 8;
const PER_ROW: u32 = 4;
const CELL: u32 = 16 * SCALE + 8;

fn main() -> ExitCode {
    let Some(out) = std::env::args().nth(1) else {
        eprintln!("usage: fallback_preview <out.bmp>");
        return ExitCode::FAILURE;
    };
    let rows = (FALLBACK_KEYS.len() as u32).div_ceil(PER_ROW);
    let (w, h) = (PER_ROW * CELL, rows * CELL);
    // Checkerboard background shows transparency.
    let mut px = vec![[0u8; 4]; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let v = if (x / 8 + y / 8) % 2 == 0 { 200 } else { 160 };
            px[(y * w + x) as usize] = [v, v, v, 255];
        }
    }
    for (i, key) in FALLBACK_KEYS.iter().enumerate() {
        let tex = fallback_texture(key);
        let (ox, oy) = (
            (i as u32 % PER_ROW) * CELL + 4,
            (i as u32 / PER_ROW) * CELL + 4,
        );
        for y in 0..16 * SCALE {
            for x in 0..16 * SCALE {
                let s = tex.get(x / SCALE, y / SCALE);
                let d = &mut px[((oy + y) * w + ox + x) as usize];
                let a = u32::from(s[3]);
                for c in 0..3 {
                    d[c] = ((u32::from(s[c]) * a + u32::from(d[c]) * (255 - a)) / 255) as u8;
                }
            }
        }
    }
    // 24-bit BMP, bottom-up rows padded to 4 bytes.
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut bmp = Vec::with_capacity(size as usize);
    bmp.extend_from_slice(b"BM");
    for v in [size, 0, 54, 40] {
        bmp.extend_from_slice(&v.to_le_bytes());
    }
    for v in [w as i32, h as i32] {
        bmp.extend_from_slice(&v.to_le_bytes());
    }
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    for v in [0u32, row * h, 2835, 2835, 0, 0] {
        bmp.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..h).rev() {
        let start = bmp.len();
        for x in 0..w {
            let p = px[(y * w + x) as usize];
            bmp.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        bmp.resize(start + row as usize, 0);
    }
    if let Err(e) = std::fs::write(&out, bmp) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    println!("wrote {out}: {}", FALLBACK_KEYS.join(", "));
    ExitCode::SUCCESS
}
