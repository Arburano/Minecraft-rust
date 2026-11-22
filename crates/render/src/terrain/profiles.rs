//! Quality profiles (`meshing-and-rendering.md`, "Render graph and
//! profiles"): render distance never below 8 chunks (PRD §14).

use serde::{Deserialize, Serialize};

/// A named set of render settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityProfile {
    /// 8 chunks, optional passes off.
    Low,
    /// 12 chunks.
    Default,
    /// 16 chunks, fog fades later.
    High,
}

/// Settings derived from a profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSettings {
    /// Full-data render distance in chunks (16 blocks).
    pub render_distance: u32,
    /// Fraction of the render distance where fog starts.
    pub fog_start: f32,
    /// Mipmapped texture sampling (off = nearest only).
    pub mipmaps: bool,
    /// Cave culling (always on; kept as a switch for profiling).
    pub cave_culling: bool,
}

impl QualityProfile {
    /// The profile's settings.
    #[must_use]
    pub fn settings(self) -> RenderSettings {
        match self {
            Self::Low => RenderSettings {
                render_distance: 8,
                fog_start: 0.6,
                mipmaps: false,
                cave_culling: true,
            },
            Self::Default => RenderSettings {
                render_distance: 12,
                fog_start: 0.7,
                mipmaps: true,
                cave_culling: true,
            },
            Self::High => RenderSettings {
                render_distance: 16,
                fog_start: 0.8,
                mipmaps: true,
                cave_culling: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_and_floor() {
        assert_eq!(QualityProfile::Default.settings().render_distance, 12);
        for p in [
            QualityProfile::Low,
            QualityProfile::Default,
            QualityProfile::High,
        ] {
            assert!(
                p.settings().render_distance >= 8,
                "PRD §14: never below 8 chunks"
            );
        }
    }
}
