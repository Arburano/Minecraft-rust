//! Naive reference lighting: repeated relaxation sweeps until nothing changes.
//!
//! Deliberately shares no code with the BFS implementation: it recomputes the
//! sky sources from the opacities and iterates the defining equation from
//! `neighborhood.rs` over plain `(x, y, z)` loops with bounds checks. Slow
//! (tens of sweeps over 48^3); for tests only.

use crate::neighborhood::{EDGE, LightProps, VOLUME};

/// Block and sky light (YZX over 48^3) for the given cells
/// (`cells[(y*48+z)*48+x]`) and per-column open-sky flags (`open_top[z*48+x]`).
///
/// # Panics
/// If `cells.len() != 48^3` or `open_top.len() != 48^2`.
#[must_use]
pub fn naive_light(cells: &[LightProps], open_top: &[bool]) -> (Vec<u8>, Vec<u8>) {
    assert_eq!(cells.len(), VOLUME, "cells must cover the 48^3 snapshot");
    assert_eq!(open_top.len(), EDGE * EDGE, "one flag per column");
    let at = |x: usize, y: usize, z: usize| (y * EDGE + z) * EDGE + x;

    let block_src: Vec<u8> = cells.iter().map(|c| c.emission.min(15)).collect();
    let mut sky_src = vec![0u8; VOLUME];
    for z in 0..EDGE {
        for x in 0..EDGE {
            if !open_top[z * EDGE + x] {
                continue;
            }
            for y in (0..EDGE).rev() {
                if cells[at(x, y, z)].opacity > 0 {
                    break;
                }
                sky_src[at(x, y, z)] = 15;
            }
        }
    }

    let relax = |src: &[u8]| -> Vec<u8> {
        let mut light = src.to_vec();
        loop {
            let mut changed = false;
            for y in 0..EDGE {
                for z in 0..EDGE {
                    for x in 0..EDGE {
                        let i = at(x, y, z);
                        let mut best = i32::from(light[i]);
                        let mut consider = |n: usize| {
                            let cand =
                                i32::from(light[n]) - 1 - i32::from(cells[i].opacity.min(15));
                            best = best.max(cand);
                        };
                        if x > 0 {
                            consider(at(x - 1, y, z));
                        }
                        if x + 1 < EDGE {
                            consider(at(x + 1, y, z));
                        }
                        if y > 0 {
                            consider(at(x, y - 1, z));
                        }
                        if y + 1 < EDGE {
                            consider(at(x, y + 1, z));
                        }
                        if z > 0 {
                            consider(at(x, y, z - 1));
                        }
                        if z + 1 < EDGE {
                            consider(at(x, y, z + 1));
                        }
                        let best = best.clamp(0, 15) as u8;
                        if best != light[i] {
                            light[i] = best;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                return light;
            }
        }
    };
    (relax(&block_src), relax(&sky_src))
}
