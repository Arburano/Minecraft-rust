//! Property tests: greedy `fast_mesh` output must cover exactly the visible
//! faces of the naive reference mesher, on random 32^3 mesh chunks with
//! random padding, and incremental mask updates must equal a full rebuild.
//! Render batch packing (T0-05): decoded quad positions must equal the
//! chunk position plus the slot offset (mod 64), and batch faces must equal
//! the chunk faces shifted by the offset.

use mesh::reference::{batch_faces, covered_faces_sorted, naive_faces_sorted};
use mesh::{
    MaskCache, MeshChunkBuilder, MeshChunkQuads, P3, PackedBatch, Quad, RenderBatch,
    SectionNeighborhood, pad_index,
};
use proptest::prelude::*;
use world::coords::MeshChunkPos;
use world::{BlockStateId, SECTION_VOLUME, Section};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn is_transparent(mask: u8) -> impl Fn(BlockStateId) -> bool {
    move |b: BlockStateId| (5..=7).contains(&b.0) && (mask >> (b.0 - 5)) & 1 == 1
}

/// Random padded 34^3 field: ids 0..=7, several shapes so both mesher paths
/// (opaque columns and per-transparent-block) are exercised.
fn gen_field(pattern: u8, density: u8, seed: u64) -> Box<[u16; P3]> {
    let mut v = Box::new([0u16; P3]);
    let mut r = Rng(seed | 1);
    let p = 34usize;
    match pattern {
        0 => v.fill((seed % 8) as u16),
        1 => {
            for cell in v.iter_mut() {
                *cell = if (r.next() % 100) < u64::from(density) {
                    1 + (r.next() % 7) as u16
                } else {
                    0
                };
            }
        }
        2 => {
            // Blobby opaque/transparent clusters.
            for _ in 0..(u64::from(density) * 30) {
                let (x, y, z) = (
                    (r.next() % 34) as usize,
                    (r.next() % 34) as usize,
                    (r.next() % 34) as usize,
                );
                let id = 1 + (r.next() % 7) as u16;
                for (dx, dy, dz) in [
                    (0i32, 0i32, 0i32),
                    (1, 0, 0),
                    (0, 1, 0),
                    (0, 0, 1),
                    (-1, 0, 0),
                    (0, -1, 0),
                    (0, 0, -1),
                ] {
                    let (xx, yy, zz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
                    if xx >= 0 && yy >= 0 && zz >= 0 && xx < 34 && yy < 34 && zz < 34 {
                        v[(yy as usize * p + xx as usize) * p + zz as usize] = id;
                    }
                }
            }
        }
        _ => {
            // Terrain-like columns with a transparent surface layer.
            for x in 0..p {
                for z in 0..p {
                    let h = (r.next() % 34) as usize;
                    for y in 0..p {
                        let id = if y < h.saturating_sub(3) {
                            1
                        } else if y < h {
                            2
                        } else if y == h {
                            if (r.next() % 100) < u64::from(density) {
                                5 + (r.next() % 3) as u16
                            } else {
                                3
                            }
                        } else if y < 20 {
                            5
                        } else {
                            0
                        };
                        v[(y * p + x) * p + z] = id;
                    }
                }
            }
        }
    }
    v
}

/// Cuts the 64 sections of chunk (0,0,0) out of the padded field; cells
/// outside the 34^3 box are air (the gather must not read them).
fn neighborhood(field: &[u16; P3]) -> SectionNeighborhood {
    SectionNeighborhood::from_fn(MeshChunkPos { x: 0, y: 0, z: 0 }, |sp| {
        let mut arr = [BlockStateId::AIR; SECTION_VOLUME];
        for ly in 0..16i32 {
            for lz in 0..16i32 {
                for lx in 0..16i32 {
                    let (bx, by, bz) = (sp.x * 16 + lx, sp.y * 16 + ly, sp.z * 16 + lz);
                    if (-1..=32).contains(&bx) && (-1..=32).contains(&by) && (-1..=32).contains(&bz)
                    {
                        let i = pad_index((bx + 1) as usize, (by + 1) as usize, (bz + 1) as usize);
                        arr[((ly * 16 + lz) * 16 + lx) as usize] = BlockStateId(field[i]);
                    }
                }
            }
        }
        Section::from_blocks(&arr)
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Greedy quads cover exactly the reference's visible faces.
    #[test]
    fn fast_mesh_covers_reference_faces(pattern in 0u8..4, density in 0u8..=100, seed: u64, mask: u8) {
        let field = gen_field(pattern, density, seed);
        let n = neighborhood(&field);
        let pred = is_transparent(mask);
        let mut builder = MeshChunkBuilder::new();
        let quads = builder.build(&n, &pred);
        prop_assert_eq!(builder.voxels(), &*field, "gather must reproduce the field");
        let got = covered_faces_sorted(&quads);
        let want = naive_faces_sorted(builder.voxels(), &pred);
        prop_assert_eq!(got.len(), want.len(), "face count mismatch");
        prop_assert!(got == want, "covered faces differ from the reference");
    }

    /// After random edits through `set_block`, masks equal a full rebuild
    /// and the re-mesh still covers exactly the reference faces.
    #[test]
    fn incremental_edits_match_rebuild(pattern in 0u8..4, density in 0u8..=100, seed: u64, mask: u8, edits in 1usize..40) {
        let field = gen_field(pattern, density, seed);
        let n = neighborhood(&field);
        let pred = is_transparent(mask);
        let mut builder = MeshChunkBuilder::new();
        builder.gather(&n, &pred);
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        for _ in 0..edits {
            let (x, y, z) = (
                (r.next() % 32) as usize,
                (r.next() % 32) as usize,
                (r.next() % 32) as usize,
            );
            builder.set_block(x, y, z, BlockStateId((r.next() % 8) as u16), &pred);
        }
        let mut rebuilt = MaskCache::new();
        rebuilt.rebuild(builder.voxels(), &pred);
        prop_assert_eq!(builder.masks().opaque(), rebuilt.opaque());
        prop_assert_eq!(builder.masks().transparent(), rebuilt.transparent());
        let mut quads = MeshChunkQuads::default();
        builder.remesh_into(&mut quads);
        let got = covered_faces_sorted(&quads);
        let want = naive_faces_sorted(builder.voxels(), &pred);
        prop_assert!(got == want, "re-mesh after edits differs from the reference");
    }
}

/// Anchor layout per face direction, from the crate's `Face::vertices_packed`
/// geometry: plane axis, width axis, height axis, and whether the width
/// extent is subtracted from the anchor (max corner on the width axis).
const PLANE_AXIS: [usize; 6] = [1, 1, 0, 0, 2, 2];
const W_AXIS: [usize; 6] = [0, 0, 1, 1, 0, 0];
const H_AXIS: [usize; 6] = [2, 2, 2, 2, 1, 1];
const W_MAX: [bool; 6] = [false, true, true, false, true, false];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// T0-05 acceptance: the decoded position of each quad in the batch
    /// equals its position in the mesh chunk plus the slot offset (mod 64);
    /// width, height and id are preserved; packing concatenates the slots.
    #[test]
    fn batch_positions_equal_chunk_plus_offset(seed: u64, n_quads in 0usize..8) {
        let mut r = Rng(seed | 1);
        let mut quads = MeshChunkQuads::default();
        let mut anchors = [[0u64; 3]; 6 * 8];
        for dir in 0..6 {
            for k in 0..n_quads {
                let (pa, wa, ha) = (PLANE_AXIS[dir], W_AXIS[dir], H_AXIS[dir]);
                let plane = r.next() % 32;
                let w = 1 + r.next() % 32;
                let h = 1 + r.next() % 32;
                let u_min = r.next() % (33 - w);
                let v_min = r.next() % (33 - h);
                let id = r.next() % 65536;
                let mut a = [0u64; 3];
                a[pa] = plane + u64::from(dir % 2 == 0);
                a[wa] = u_min + if W_MAX[dir] { w } else { 0 };
                a[ha] = v_min;
                anchors[dir * 8 + k] = a;
                quads.quads[dir].push(Quad::pack(
                    a[0] as usize, a[1] as usize, a[2] as usize,
                    w as usize, h as usize, id as usize,
                ));
            }
        }
        let mut batch = RenderBatch::new();
        for s in 0..8 {
            batch.set_chunk(s, &quads);
        }
        prop_assert_eq!(batch.len(), 8 * quads.len());
        for s in 0..8usize {
            let (sy, sz, sx) = (s / 4, (s / 2) % 2, s % 2);
            let off = [(sx * 32) as u64, (sy * 32) as u64, (sz * 32) as u64];
            for dir in 0..6 {
                for (k, &q) in batch.chunk(s).quads[dir].iter().enumerate() {
                    let src = quads.quads[dir][k];
                    let want = [
                        (anchors[dir * 8 + k][0] + off[0]) % 64,
                        (anchors[dir * 8 + k][1] + off[1]) % 64,
                        (anchors[dir * 8 + k][2] + off[2]) % 64,
                    ];
                    prop_assert_eq!(q.xyz(), want, "slot {} dir {} quad {}", s, dir, k);
                    prop_assert_eq!(q.width(), src.width());
                    prop_assert_eq!(q.height(), src.height());
                    prop_assert_eq!(q.voxel_id(), src.voxel_id());
                }
            }
        }
        let mut packed = PackedBatch::default();
        batch.pack_into(&mut packed);
        prop_assert_eq!(packed.len(), batch.len());
        for dir in 0..6 {
            let want: Vec<Quad> = (0..8)
                .flat_map(|s| batch.chunk(s).quads[dir].iter().copied())
                .collect();
            prop_assert_eq!(&packed.quads[dir], &want);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Real mesher output placed in all 8 slots: the batch's covered unit
    /// faces equal the chunk's faces shifted by each slot's offset (exact
    /// block coordinates, including the wrapped 64 planes).
    #[test]
    fn batch_faces_equal_shifted_chunk_faces(pattern in 0u8..4, density in 0u8..=100, seed: u64, mask: u8) {
        let field = gen_field(pattern, density, seed);
        let n = neighborhood(&field);
        let mut builder = MeshChunkBuilder::new();
        let quads = builder.build(&n, is_transparent(mask));
        let base = covered_faces_sorted(&quads);
        let mut batch = RenderBatch::new();
        let mut want = Vec::with_capacity(base.len() * 8);
        for s in 0..8usize {
            let (sy, sz, sx) = (s / 4, (s / 2) % 2, s % 2);
            batch.set_chunk(s, &quads);
            want.extend(base.iter().map(|f| mesh::reference::UnitFace {
                x: f.x + (sx * 32) as u8,
                y: f.y + (sy * 32) as u8,
                z: f.z + (sz * 32) as u8,
                ..*f
            }));
        }
        let mut packed = PackedBatch::default();
        batch.pack_into(&mut packed);
        let mut got = batch_faces(&packed.quads);
        got.sort_unstable();
        want.sort_unstable();
        prop_assert!(got == want, "batch faces differ from shifted chunk faces");
    }
}
