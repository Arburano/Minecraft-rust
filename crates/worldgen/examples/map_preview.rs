//! Top-down map of a generated area as a BMP (one pixel per block, colored by
//! the topmost block, shaded by height): `cargo run --release -p worldgen
//! --example map_preview -- <out.bmp> [seed] [radius in columns]`.

use std::process::ExitCode;

use world::registry::blocks_from_dir;
use world::{BlockStateId, ColumnPos, RegistryBuilder, WorldBounds};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

fn color(name: &str) -> [u8; 3] {
    match name {
        "core:grass_block" => [95, 159, 53],
        "core:sand" => [219, 207, 163],
        "core:water" => [47, 87, 196],
        "core:oak_leaves" => [40, 100, 30],
        "core:oak_log" => [109, 85, 50],
        "core:stone" | "core:cobblestone" => [125, 125, 125],
        "core:gravel" => [131, 127, 126],
        "core:dirt" => [134, 96, 67],
        _ => [255, 0, 255],
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(out) = args.next() else {
        eprintln!("usage: map_preview <out.bmp> [seed] [radius]");
        return ExitCode::FAILURE;
    };
    let seed: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(7);
    let r: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(16);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    let mut b = RegistryBuilder::new();
    let Ok(blocks) = blocks_from_dir(&dir.join("blocks")) else {
        return ExitCode::FAILURE;
    };
    b.add_mod("core", blocks);
    let Ok(reg) = b.build() else {
        return ExitCode::FAILURE;
    };
    let Ok(src) = std::fs::read_to_string(dir.join("worldgen/overworld.toml")) else {
        return ExitCode::FAILURE;
    };
    let Ok(cfg) = WorldgenConfig::from_toml(&src, &reg) else {
        return ExitCode::FAILURE;
    };
    let ctx = GenContext::new(seed, cfg, WorldBounds::default());
    let p = Pipeline::overworld();
    let side = (2 * r * 16) as u32;
    let mut px = vec![[0u8; 3]; (side * side) as usize];
    for cz in -r..r {
        for cx in -r..r {
            let col = p.generate(&ctx, ColumnPos { x: cx, z: cz });
            for z in 0..16 {
                for x in 0..16 {
                    let mut y = 319;
                    while y > -64 && col.get(x, y, z) == BlockStateId::AIR {
                        y -= 1;
                    }
                    let name = reg.name(col.get(x, y, z)).unwrap_or("?");
                    let shade = ((y - 62) as f32 * 2.0).clamp(-60.0, 60.0);
                    let c = color(name).map(|v| (f32::from(v) + shade).clamp(0.0, 255.0) as u8);
                    let (ix, iz) = (
                        ((cx + r) * 16) as u32 + x as u32,
                        ((cz + r) * 16) as u32 + z as u32,
                    );
                    px[(iz * side + ix) as usize] = c;
                }
            }
        }
    }
    let row = (side * 3).div_ceil(4) * 4;
    let size = 54 + row * side;
    let mut bmp = Vec::with_capacity(size as usize);
    bmp.extend_from_slice(b"BM");
    for v in [size, 0, 54, 40] {
        bmp.extend_from_slice(&v.to_le_bytes());
    }
    for v in [side as i32, side as i32] {
        bmp.extend_from_slice(&v.to_le_bytes());
    }
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    for v in [0u32, row * side, 2835, 2835, 0, 0] {
        bmp.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..side).rev() {
        let start = bmp.len();
        for x in 0..side {
            let p = px[(y * side + x) as usize];
            bmp.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        bmp.resize(start + row as usize, 0);
    }
    if std::fs::write(&out, bmp).is_err() {
        return ExitCode::FAILURE;
    }
    println!("wrote {out}: seed {seed}, {side}x{side} blocks");
    ExitCode::SUCCESS
}
