//! GPU capability reporting and stage 0–1 rendering path selection.

pub mod selection;

use serde::Serialize;
use wgpu::{Adapter, AdapterInfo, DownlevelCapabilities, DownlevelFlags, Features, Limits};

/// Rendering paths implemented before GPU-driven culling in stage 2.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum RenderPath {
    /// Path B: six vertices per quad, pulled from vertex-stage storage.
    #[serde(rename = "B")]
    VertexPulling,
    /// Path C: four triangle-strip vertices per 8-byte instance.
    #[serde(rename = "C")]
    Instanced,
}

/// Selects B when vertex storage is available, otherwise C; never selects A.
#[must_use]
pub const fn select_path(flags: DownlevelFlags) -> RenderPath {
    if flags.contains(DownlevelFlags::VERTEX_STORAGE) {
        RenderPath::VertexPulling
    } else {
        RenderPath::Instanced
    }
}

/// Serializable adapter capabilities, including the selected prototype path.
#[derive(Debug, Serialize)]
pub struct AdapterReport {
    /// Adapter index within its backend's enumeration, not a global GPU index.
    pub adapter_index: usize,
    /// GPU name, backend, hardware identifiers and driver information.
    pub info: AdapterInfo,
    /// Optional features exposed by the adapter.
    pub features: Features,
    /// Resource and shader limits exposed by the adapter.
    pub limits: Limits,
    /// Downlevel flags, limits and shader model exposed by the adapter.
    pub downlevel_capabilities: DownlevelCapabilities,
    /// Stage 0–1 path selected from the downlevel flags.
    pub selected_path: RenderPath,
}

impl AdapterReport {
    /// Captures capabilities without requesting a device or rendering a frame.
    #[must_use]
    pub fn from_adapter(adapter: &Adapter, adapter_index: usize) -> Self {
        let downlevel_capabilities = adapter.get_downlevel_capabilities();
        Self {
            adapter_index,
            info: adapter.get_info(),
            features: adapter.features(),
            limits: adapter.limits(),
            selected_path: select_path(downlevel_capabilities.flags),
            downlevel_capabilities,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_capabilities_use_instances() {
        assert_eq!(select_path(DownlevelFlags::empty()), RenderPath::Instanced);
    }

    #[test]
    fn vertex_storage_alone_is_sufficient() {
        assert_eq!(
            select_path(DownlevelFlags::VERTEX_STORAGE),
            RenderPath::VertexPulling
        );
    }

    #[test]
    fn full_capabilities_still_select_b_not_a() {
        assert_eq!(
            select_path(DownlevelFlags::all()),
            RenderPath::VertexPulling
        );
    }

    #[test]
    fn compute_and_indirect_do_not_replace_vertex_storage() {
        let flags = DownlevelFlags::all() - DownlevelFlags::VERTEX_STORAGE;
        assert_eq!(select_path(flags), RenderPath::Instanced);
    }

    #[test]
    fn each_unrelated_flag_leaves_selection_unchanged() {
        for flag in DownlevelFlags::all().iter() {
            if flag != DownlevelFlags::VERTEX_STORAGE {
                assert_eq!(select_path(flag), RenderPath::Instanced);
                assert_eq!(
                    select_path(flag | DownlevelFlags::VERTEX_STORAGE),
                    RenderPath::VertexPulling
                );
            }
        }
    }

    #[test]
    fn paths_serialize_as_specification_labels() -> Result<(), serde_json::Error> {
        assert_eq!(serde_json::to_string(&RenderPath::VertexPulling)?, "\"B\"");
        assert_eq!(serde_json::to_string(&RenderPath::Instanced)?, "\"C\"");
        Ok(())
    }
}
