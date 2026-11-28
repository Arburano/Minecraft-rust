//! Built-in overworld stages, configured by mod data (`config.rs`).

use noise::NoiseFn;

use crate::config::{TreeShape, Trees};
use crate::pipeline::{GenContext, ProtoColumn, Stage};

/// Cheap 2D pass: surface height and biome of every block column.
pub struct SurfacePass;

impl Stage for SurfacePass {
    fn id(&self) -> &str {
        "core:surface"
    }

    fn run(&self, ctx: &GenContext, col: &mut ProtoColumn) {
        let (bx, bz) = (col.pos.x * 16, col.pos.z * 16);
        for z in 0..16 {
            for x in 0..16 {
                col.surface[z * 16 + x] = ctx.surface(bx + x as i32, bz + z as i32);
            }
        }
    }
}

/// Stone up to the surface, biome top and filler layers, beaches, sea water.
pub struct TerrainFill;

impl Stage for TerrainFill {
    fn id(&self) -> &str {
        "core:terrain"
    }

    fn run(&self, ctx: &GenContext, col: &mut ProtoColumn) {
        let cfg = &ctx.config;
        let min_y = col.bounds().min_y();
        for z in 0..16 {
            for x in 0..16 {
                let s = col.surface[z * 16 + x];
                let biome = &cfg.biomes[usize::from(s.biome)];
                let beach = s.height <= cfg.sea_level + cfg.beach_max_above_sea;
                let (top, filler) = if beach {
                    (cfg.beach, cfg.beach)
                } else {
                    (biome.top, biome.filler)
                };
                for y in min_y..=s.height {
                    let id = if y == s.height {
                        top
                    } else if y >= s.height - biome.filler_depth {
                        filler
                    } else {
                        cfg.terrain.stone
                    };
                    col.set(x, y, z, id);
                }
                for y in s.height + 1..=cfg.sea_level {
                    col.set(x, y, z, cfg.terrain.fluid);
                }
            }
        }
    }
}

/// 3D-noise caves, sampled on a 4-block grid and interpolated trilinearly
/// (16× fewer noise samples than per block).
pub struct CaveCarver;

const CELL: i32 = 4;

impl Stage for CaveCarver {
    fn id(&self) -> &str {
        "core:caves"
    }

    fn run(&self, ctx: &GenContext, col: &mut ProtoColumn) {
        let c = &ctx.config.caves;
        let top = col
            .surface
            .iter()
            .map(|s| s.height)
            .max()
            .unwrap_or(c.min_y);
        let y0 = c.min_y.max(col.bounds().min_y()).div_euclid(CELL) * CELL;
        let y1 = top - c.max_below_surface;
        if y1 <= y0 {
            return;
        }
        let ny = ((y1 - y0) / CELL + 2) as usize;
        // Noise at grid corners: 5 x ny x 5.
        let (bx, bz) = (col.pos.x * 16, col.pos.z * 16);
        let mut grid = vec![0.0f64; 5 * ny * 5];
        for gy in 0..ny {
            for gz in 0..5 {
                for gx in 0..5 {
                    let p = [
                        f64::from(bx + gx as i32 * CELL) * c.scale,
                        f64::from(y0 + gy as i32 * CELL) * c.scale * 1.6,
                        f64::from(bz + gz as i32 * CELL) * c.scale,
                    ];
                    grid[(gy * 5 + gz) * 5 + gx] = ctx.caves.get(p);
                }
            }
        }
        let at = |gx: usize, gy: usize, gz: usize| grid[(gy * 5 + gz) * 5 + gx];
        let fluid = ctx.config.terrain.fluid;
        for z in 0..16 {
            for x in 0..16 {
                let limit = col.surface[z * 16 + x].height - c.max_below_surface;
                let (gx, fx) = (x / 4, (x % 4) as f64 / 4.0);
                let (gz, fz) = (z / 4, (z % 4) as f64 / 4.0);
                for y in y0.max(c.min_y)..limit.min(y1 + 1) {
                    let gy = ((y - y0) / CELL) as usize;
                    let fy = f64::from((y - y0) % CELL) / f64::from(CELL);
                    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
                    let plane = |gy: usize| {
                        lerp(
                            lerp(at(gx, gy, gz), at(gx + 1, gy, gz), fx),
                            lerp(at(gx, gy, gz + 1), at(gx + 1, gy, gz + 1), fx),
                            fz,
                        )
                    };
                    let n = lerp(plane(gy), plane(gy + 1), fy);
                    if n.abs() > c.threshold && col.get(x, y, z) != fluid {
                        col.set(x, y, z, c.air);
                    }
                }
            }
        }
    }
}

/// Ore veins as short random walks inside the column.
pub struct OreVeins;

impl Stage for OreVeins {
    fn id(&self) -> &str {
        "core:ores"
    }

    fn run(&self, ctx: &GenContext, col: &mut ProtoColumn) {
        let (cx, cz) = (i64::from(col.pos.x), i64::from(col.pos.z));
        for (k, ore) in ctx.config.ores.iter().enumerate() {
            for v in 0..ore.veins_per_column {
                let mut rng = ctx.rng(&[0x0e, cx, cz, k as i64, i64::from(v)]);
                let (mut x, mut y, mut z) = (
                    rng.range(0, 16),
                    rng.range(ore.min_y, ore.max_y),
                    rng.range(0, 16),
                );
                for _ in 0..ore.vein_size {
                    if (0..16).contains(&x) && (0..16).contains(&z) {
                        let (ux, uz) = (x as usize, z as usize);
                        if col.get(ux, y, uz) == ore.replaces {
                            col.set(ux, y, uz, ore.block);
                        }
                    }
                    match rng.next_u64() % 6 {
                        0 => x += 1,
                        1 => x -= 1,
                        2 => y += 1,
                        3 => y -= 1,
                        4 => z += 1,
                        _ => z -= 1,
                    }
                }
            }
        }
    }
}

/// Trees. Each column places the trees whose origins lie in itself and its 8
/// neighbors (a crown reaches 2 blocks), writing only its own cells, so trees
/// across column borders are identical from both sides.
pub struct TreePlacer;

impl Stage for TreePlacer {
    fn id(&self) -> &str {
        "core:trees"
    }

    fn run(&self, ctx: &GenContext, col: &mut ProtoColumn) {
        let cfg = &ctx.config;
        for ncz in col.pos.z - 1..=col.pos.z + 1 {
            for ncx in col.pos.x - 1..=col.pos.x + 1 {
                let mut rng = ctx.rng(&[0x7e, i64::from(ncx), i64::from(ncz)]);
                // Density from the biome at the neighbor column's center.
                let center = ctx.surface(ncx * 16 + 8, ncz * 16 + 8);
                let density = cfg.biomes[usize::from(center.biome)].trees_per_column;
                let mut count = density.floor() as u32;
                if rng.unit() < density.fract() {
                    count += 1;
                }
                for _ in 0..count {
                    let (ox, oz) = (ncx * 16 + rng.range(0, 16), ncz * 16 + rng.range(0, 16));
                    let s = ctx.surface(ox, oz);
                    let biome = &cfg.biomes[usize::from(s.biome)];
                    // The kind growing at the origin; no random draw when a
                    // biome has one kind, so single-kind data keeps its trees.
                    let kind = match biome.trees.len() {
                        0 => 0,
                        1 => usize::from(biome.trees[0]),
                        n => usize::from(biome.trees[rng.range(0, n as i32) as usize]),
                    };
                    let t = &cfg.tree_kinds[kind];
                    let trunk = rng.range(t.min_trunk, t.max_trunk + 1);
                    let beach = s.height <= cfg.sea_level + cfg.beach_max_above_sea;
                    if beach || biome.trees_per_column <= 0.0 {
                        continue;
                    }
                    place_tree(col, ctx, t, ox, s.height + 1, oz, trunk);
                }
            }
        }
    }
}

/// Writes the leaves of one crown layer (radius `r` ≤ 2) that fall inside
/// this column, only into air.
fn leaf_layer(
    col: &mut ProtoColumn,
    ctx: &GenContext,
    leaves: world::BlockStateId,
    (ox, y, oz): (i32, i32, i32),
    r: i32,
    keep_corners: bool,
) {
    let (bx, bz) = (col.pos.x * 16, col.pos.z * 16);
    for dz in -r..=r {
        for dx in -r..=r {
            if r > 0 && dx.abs() == r && dz.abs() == r && !keep_corners {
                continue; // round the corners
            }
            let (lx, lz) = (ox + dx - bx, oz + dz - bz);
            if (0..16).contains(&lx) && (0..16).contains(&lz) {
                let (lx, lz) = (lx as usize, lz as usize);
                if col.get(lx, y, lz) == ctx.config.caves.air {
                    col.set(lx, y, lz, leaves);
                }
            }
        }
    }
}

fn place_tree(
    col: &mut ProtoColumn,
    ctx: &GenContext,
    t: &Trees,
    ox: i32,
    base: i32,
    oz: i32,
    trunk: i32,
) {
    let (bx, bz) = (col.pos.x * 16, col.pos.z * 16);
    let top = base + trunk;
    // Crown first, only into air, then the trunk over it.
    match t.shape {
        TreeShape::Round => {
            for dy in -2i32..=1 {
                let r: i32 = if dy >= 0 { 1 } else { 2 };
                leaf_layer(col, ctx, t.leaves, (ox, top + dy, oz), r, dy == -1);
            }
        }
        TreeShape::Cone => {
            // A tip above the trunk, then rings of radius 1 and 2 alternating
            // down to two blocks above the ground.
            leaf_layer(col, ctx, t.leaves, (ox, top, oz), 0, false);
            for (i, y) in (base + 2..top).rev().enumerate() {
                let r = if i % 2 == 0 { 1 } else { 2 };
                leaf_layer(col, ctx, t.leaves, (ox, y, oz), r, false);
            }
        }
    }
    let (lx, lz) = (ox - bx, oz - bz);
    if (0..16).contains(&lx) && (0..16).contains(&lz) {
        for y in base..top {
            col.set(lx as usize, y, lz as usize, t.log);
        }
    }
}
