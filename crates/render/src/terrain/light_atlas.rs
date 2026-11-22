//! Light on the GPU: sections' light as 16³ bricks in a 3D atlas texture plus
//! an indirection volume (`meshing-and-rendering.md`: "the shader reads light
//! from the sections' 3D light texture").
//!
//! Indirection entry per section (toroidal addressing by section position):
//! bit 31 set → a brick slot in bits 0..30; clear → a uniform section with
//! block light in bits 0..3 and sky light in bits 4..7 (no brick needed, the
//! common case for open sky and solid rock). Texel value: `block | sky << 4`.

use std::collections::HashMap;

use glam::IVec3;

/// Bricks per atlas axis (x, y, z): 32 × 32 × 8 = 8,192 bricks of 16³,
/// a 512 × 512 × 128 R8 texture (32 MiB).
pub const ATLAS_BRICKS: [u32; 3] = [32, 32, 8];
/// Indirection volume in sections (x, y, z), toroidal.
pub const INDIRECTION: [u32; 3] = [64, 32, 64];
/// Flag of an indirection entry that points at a brick.
pub const BRICK_FLAG: u32 = 1 << 31;

/// One section's light as the renderer needs it.
pub enum SectionLightData<'a> {
    /// Every cell has these levels.
    Uniform {
        /// Block light 0..15.
        block: u8,
        /// Sky light 0..15.
        sky: u8,
    },
    /// 4,096 texels `block | sky << 4`, YZX order.
    Texels(&'a [u8; 4096]),
}

/// Slot bookkeeping (CPU side). The renderer writes bricks and entries.
pub struct LightAtlas {
    free: Vec<u32>,
    slots: HashMap<IVec3, u32>,
}

impl Default for LightAtlas {
    fn default() -> Self {
        Self::new()
    }
}

impl LightAtlas {
    /// All slots free.
    #[must_use]
    pub fn new() -> Self {
        let n = ATLAS_BRICKS[0] * ATLAS_BRICKS[1] * ATLAS_BRICKS[2];
        Self {
            free: (0..n).rev().collect(),
            slots: HashMap::new(),
        }
    }

    /// Texel origin of a brick slot inside the atlas.
    #[must_use]
    pub fn slot_origin(slot: u32) -> [u32; 3] {
        let (bx, by) = (ATLAS_BRICKS[0], ATLAS_BRICKS[1]);
        [
            (slot % bx) * 16,
            (slot / bx % by) * 16,
            (slot / (bx * by)) * 16,
        ]
    }

    /// Indirection texel of a section (toroidal).
    #[must_use]
    pub fn indirection_index(section: IVec3) -> [u32; 3] {
        [
            section.x.rem_euclid(INDIRECTION[0] as i32) as u32,
            section.y.rem_euclid(INDIRECTION[1] as i32) as u32,
            section.z.rem_euclid(INDIRECTION[2] as i32) as u32,
        ]
    }

    /// Updates a section: returns the indirection entry and, for non-uniform
    /// light, the slot whose brick must be (re)uploaded. `None` slot with a
    /// uniform entry frees the section's old brick. Errors when full.
    pub fn update(
        &mut self,
        section: IVec3,
        data: &SectionLightData<'_>,
    ) -> Result<(u32, Option<u32>), AtlasFull> {
        match data {
            SectionLightData::Uniform { block, sky } => {
                if let Some(slot) = self.slots.remove(&section) {
                    self.free.push(slot);
                }
                Ok((u32::from(block & 15) | (u32::from(sky & 15) << 4), None))
            }
            SectionLightData::Texels(_) => {
                let slot = match self.slots.get(&section) {
                    Some(&s) => s,
                    None => {
                        let s = self.free.pop().ok_or(AtlasFull)?;
                        self.slots.insert(section, s);
                        s
                    }
                };
                Ok((BRICK_FLAG | slot, Some(slot)))
            }
        }
    }

    /// Frees an unloaded section's brick.
    pub fn remove(&mut self, section: IVec3) {
        if let Some(slot) = self.slots.remove(&section) {
            self.free.push(slot);
        }
    }

    /// Bricks in use.
    #[must_use]
    pub fn used(&self) -> usize {
        self.slots.len()
    }
}

/// The atlas has no free brick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasFull;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_sections_need_no_brick() {
        let mut a = LightAtlas::new();
        let texels = [0x3fu8; 4096];
        let (e, slot) = a
            .update(IVec3::new(1, 2, 3), &SectionLightData::Texels(&texels))
            .expect("room");
        assert_eq!(e & BRICK_FLAG, BRICK_FLAG);
        assert_eq!(slot, Some(e & !BRICK_FLAG));
        let (again, same) = a
            .update(IVec3::new(1, 2, 3), &SectionLightData::Texels(&texels))
            .expect("room");
        assert_eq!((again, same), (e, slot), "same slot on re-upload");
        let (u, none) = a
            .update(
                IVec3::new(1, 2, 3),
                &SectionLightData::Uniform { block: 2, sky: 15 },
            )
            .expect("ok");
        assert_eq!((u, none), (0xf2, None));
        assert_eq!(a.used(), 0, "brick freed");
        assert_eq!(LightAtlas::slot_origin(33), [16, 16, 0]);
        assert_eq!(
            LightAtlas::indirection_index(IVec3::new(-1, -1, 64)),
            [63, 31, 0]
        );
    }
}
