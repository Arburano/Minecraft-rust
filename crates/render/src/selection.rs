//! Backend and adapter selection with fallback (T1-05).
//!
//! T0-14 findings this module handles (`meshing-and-rendering.md`):
//! - Vulkan can enumerate **zero** adapters on AMD switchable-graphics laptops;
//!   players cannot be asked to set environment variables, so the next backend
//!   is tried (DX12 on Windows, then GL).
//! - Adapter order changes with driver settings (the NVIDIA preferred-GPU
//!   setting swapped indices), so adapters are chosen by type, capability and
//!   an optional user-given name, **never by index**.
//! - Device creation can fail on a listed adapter; the attempt list continues.
//!
//! The decision is a pure function over [`AdapterCandidate`]s (unit-tested in
//! CI without a GPU); [`select_device`] is the thin wgpu layer around it.

use serde::{Deserialize, Serialize};

use crate::{RenderPath, select_path};

/// Graphics API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// Vulkan.
    Vulkan,
    /// Direct3D 12 (Windows).
    Dx12,
    /// Metal (macOS).
    Metal,
    /// OpenGL / GLES.
    Gl,
}

impl BackendKind {
    /// The wgpu backend.
    #[must_use]
    pub fn to_wgpu(self) -> wgpu::Backend {
        match self {
            Self::Vulkan => wgpu::Backend::Vulkan,
            Self::Dx12 => wgpu::Backend::Dx12,
            Self::Metal => wgpu::Backend::Metal,
            Self::Gl => wgpu::Backend::Gl,
        }
    }

    /// The wgpu backend bit set for enumeration.
    #[must_use]
    pub fn to_wgpu_set(self) -> wgpu::Backends {
        match self {
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::Dx12 => wgpu::Backends::DX12,
            Self::Metal => wgpu::Backends::METAL,
            Self::Gl => wgpu::Backends::GL,
        }
    }
}

/// Operating system family, for the default backend order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsFamily {
    /// Windows.
    Windows,
    /// macOS.
    MacOs,
    /// Linux and other Unix.
    Linux,
}

impl OsFamily {
    /// The OS this binary is built for.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Default backend order: Vulkan first where available (path B with vertex
/// storage), the native API next, GL last (path C on old GPUs).
#[must_use]
pub fn backend_order(os: OsFamily) -> &'static [BackendKind] {
    match os {
        OsFamily::Windows => &[BackendKind::Vulkan, BackendKind::Dx12, BackendKind::Gl],
        OsFamily::MacOs => &[BackendKind::Metal],
        OsFamily::Linux => &[BackendKind::Vulkan, BackendKind::Gl],
    }
}

/// Adapter type, mirroring `wgpu::DeviceType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// Discrete GPU.
    Discrete,
    /// Integrated GPU.
    Integrated,
    /// Virtual GPU (VMs).
    Virtual,
    /// Software rasterizer.
    Cpu,
    /// Unknown (GL often reports this).
    Other,
}

impl From<wgpu::DeviceType> for DeviceKind {
    fn from(t: wgpu::DeviceType) -> Self {
        match t {
            wgpu::DeviceType::DiscreteGpu => Self::Discrete,
            wgpu::DeviceType::IntegratedGpu => Self::Integrated,
            wgpu::DeviceType::VirtualGpu => Self::Virtual,
            wgpu::DeviceType::Cpu => Self::Cpu,
            wgpu::DeviceType::Other => Self::Other,
        }
    }
}

/// What the selection needs to know about one enumerated adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterCandidate {
    /// Backend it was enumerated on.
    pub backend: BackendKind,
    /// Driver-reported name.
    pub name: String,
    /// Adapter type.
    pub device: DeviceKind,
    /// Render path this adapter supports (B with vertex storage, else C).
    pub path: RenderPath,
}

/// Which GPU class to prefer when several are present.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerPreference {
    /// Discrete GPU first (default: the game wants the faster GPU).
    #[default]
    HighPerformance,
    /// Integrated GPU first (battery).
    LowPower,
}

/// User override from the client settings (persisted by T1-13).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuPreference {
    /// Try this backend first; others remain as fallback.
    pub backend: Option<BackendKind>,
    /// Try adapters whose name contains this text (case-insensitive) first.
    pub adapter_name: Option<String>,
    /// GPU class preference.
    #[serde(default)]
    pub power: PowerPreference,
}

fn device_rank(device: DeviceKind, power: PowerPreference) -> u8 {
    match (power, device) {
        (PowerPreference::HighPerformance, DeviceKind::Discrete)
        | (PowerPreference::LowPower, DeviceKind::Integrated) => 0,
        (PowerPreference::HighPerformance, DeviceKind::Integrated)
        | (PowerPreference::LowPower, DeviceKind::Discrete) => 1,
        (_, DeviceKind::Other | DeviceKind::Virtual) => 2,
        (_, DeviceKind::Cpu) => 3,
    }
}

/// Ordered attempt list: indices into `candidates`, best first. The caller
/// tries to create a device on each in turn and keeps the first that works.
///
/// Order: backends by the user's choice, then `backend_order(os)`, then any
/// other backend that enumerated adapters; inside a backend, adapters matching
/// the user's name first, then by GPU class, then by name (stable, never by
/// enumeration index). Software adapters come after every hardware adapter of
/// every backend.
#[must_use]
pub fn attempt_order(
    candidates: &[AdapterCandidate],
    preference: &GpuPreference,
    os: OsFamily,
) -> Vec<usize> {
    let mut backends: Vec<BackendKind> = Vec::new();
    for b in preference
        .backend
        .iter()
        .copied()
        .chain(backend_order(os).iter().copied())
        .chain(candidates.iter().map(|c| c.backend))
    {
        if !backends.contains(&b) {
            backends.push(b);
        }
    }
    let wanted = preference.adapter_name.as_ref().map(|n| n.to_lowercase());
    let name_rank = |c: &AdapterCandidate| match &wanted {
        Some(w) if c.name.to_lowercase().contains(w.as_str()) => 0u8,
        _ => 1,
    };
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&a, &b| {
        let (ca, cb) = (&candidates[a], &candidates[b]);
        let software = |c: &AdapterCandidate| c.device == DeviceKind::Cpu;
        let backend_pos = |c: &AdapterCandidate| backends.iter().position(|&x| x == c.backend);
        (
            software(ca),
            name_rank(ca),
            backend_pos(ca),
            device_rank(ca.device, preference.power),
            &ca.name,
        )
            .cmp(&(
                software(cb),
                name_rank(cb),
                backend_pos(cb),
                device_rank(cb.device, preference.power),
                &cb.name,
            ))
    });
    order
}

/// Everything enumerated on one backend (empty when the backend is missing or
/// enumerates nothing, as Vulkan does behind the AMD switchable-graphics layer).
fn enumerate(backend: BackendKind) -> Vec<(wgpu::Adapter, AdapterCandidate)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: backend.to_wgpu_set(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    pollster::block_on(instance.enumerate_adapters(backend.to_wgpu_set()))
        .into_iter()
        .map(|adapter| {
            let info = adapter.get_info();
            let candidate = AdapterCandidate {
                backend,
                name: info.name.clone(),
                device: info.device_type.into(),
                path: select_path(adapter.get_downlevel_capabilities().flags),
            };
            (adapter, candidate)
        })
        .collect()
}

/// One failed attempt, for logs and the settings screen.
#[derive(Clone, Debug)]
pub struct FailedAttempt {
    /// The adapter that failed.
    pub candidate: AdapterCandidate,
    /// Why.
    pub error: String,
}

/// The chosen GPU.
pub struct SelectedDevice {
    /// The adapter.
    pub adapter: wgpu::Adapter,
    /// Its device.
    pub device: wgpu::Device,
    /// Its queue.
    pub queue: wgpu::Queue,
    /// What was chosen.
    pub candidate: AdapterCandidate,
    /// Attempts that failed before this one.
    pub failed: Vec<FailedAttempt>,
}

/// Enumerates every backend of this OS (plus the user's), orders the adapters
/// with [`attempt_order`] and returns the first adapter whose device can be
/// created with `descriptor`. Errors only if every attempt fails.
pub fn select_device(
    preference: &GpuPreference,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<SelectedDevice, Vec<FailedAttempt>> {
    let os = OsFamily::current();
    let mut backends: Vec<BackendKind> = preference.backend.into_iter().collect();
    for &b in backend_order(os) {
        if !backends.contains(&b) {
            backends.push(b);
        }
    }
    let mut adapters: Vec<(wgpu::Adapter, AdapterCandidate)> =
        backends.into_iter().flat_map(enumerate).collect();
    let candidates: Vec<AdapterCandidate> = adapters.iter().map(|(_, c)| c.clone()).collect();
    let order = attempt_order(&candidates, preference, os);
    let mut slots: Vec<Option<(wgpu::Adapter, AdapterCandidate)>> =
        adapters.drain(..).map(Some).collect();
    let mut failed = Vec::new();
    for i in order {
        let Some((adapter, candidate)) = slots[i].take() else {
            continue;
        };
        match pollster::block_on(adapter.request_device(descriptor)) {
            Ok((device, queue)) => {
                return Ok(SelectedDevice {
                    adapter,
                    device,
                    queue,
                    candidate,
                    failed,
                });
            }
            Err(e) => failed.push(FailedAttempt {
                candidate,
                error: e.to_string(),
            }),
        }
    }
    Err(failed)
}

/// Like [`select_device`], for a window: adapters come from `instance` (one
/// instance with every backend, so the window has one surface) and must be
/// able to present to `surface`; the order is [`attempt_order`].
pub fn select_for_surface(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
    preference: &GpuPreference,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<SelectedDevice, Vec<FailedAttempt>> {
    let os = OsFamily::current();
    let mut adapters: Vec<(wgpu::Adapter, AdapterCandidate)> =
        pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .into_iter()
            .filter(|a| a.is_surface_supported(surface))
            .filter_map(|adapter| {
                let info = adapter.get_info();
                let backend = match info.backend {
                    wgpu::Backend::Vulkan => BackendKind::Vulkan,
                    wgpu::Backend::Dx12 => BackendKind::Dx12,
                    wgpu::Backend::Metal => BackendKind::Metal,
                    wgpu::Backend::Gl => BackendKind::Gl,
                    _ => return None,
                };
                let candidate = AdapterCandidate {
                    backend,
                    name: info.name.clone(),
                    device: info.device_type.into(),
                    path: select_path(adapter.get_downlevel_capabilities().flags),
                };
                Some((adapter, candidate))
            })
            .collect();
    let candidates: Vec<AdapterCandidate> = adapters.iter().map(|(_, c)| c.clone()).collect();
    let order = attempt_order(&candidates, preference, os);
    let mut slots: Vec<Option<(wgpu::Adapter, AdapterCandidate)>> =
        adapters.drain(..).map(Some).collect();
    let mut failed = Vec::new();
    for i in order {
        let Some((adapter, candidate)) = slots[i].take() else {
            continue;
        };
        match pollster::block_on(adapter.request_device(descriptor)) {
            Ok((device, queue)) => {
                return Ok(SelectedDevice {
                    adapter,
                    device,
                    queue,
                    candidate,
                    failed,
                });
            }
            Err(e) => failed.push(FailedAttempt {
                candidate,
                error: e.to_string(),
            }),
        }
    }
    Err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(backend: BackendKind, name: &str, device: DeviceKind) -> AdapterCandidate {
        AdapterCandidate {
            backend,
            name: name.into(),
            device,
            path: RenderPath::VertexPulling,
        }
    }

    fn names(cands: &[AdapterCandidate], order: &[usize]) -> Vec<String> {
        order
            .iter()
            .map(|&i| format!("{:?}:{}", cands[i].backend, cands[i].name))
            .collect()
    }

    /// The owner's laptop (T0-14): DX12 lists 1660 Ti, Vega 10 and the
    /// software adapter; GL only one GPU; Vulkan nothing (AMD layer).
    fn laptop(order_swapped: bool) -> Vec<AdapterCandidate> {
        let mut dx12 = vec![
            c(
                BackendKind::Dx12,
                "NVIDIA GeForce GTX 1660 Ti",
                DeviceKind::Discrete,
            ),
            c(
                BackendKind::Dx12,
                "AMD Radeon(TM) RX Vega 10 Graphics",
                DeviceKind::Integrated,
            ),
        ];
        if order_swapped {
            dx12.reverse();
        }
        dx12.push(c(
            BackendKind::Dx12,
            "Microsoft Basic Render Driver",
            DeviceKind::Cpu,
        ));
        dx12.push(c(
            BackendKind::Gl,
            "NVIDIA GeForce GTX 1660 Ti/PCIe/SSE2",
            DeviceKind::Other,
        ));
        dx12
    }

    #[test]
    fn vulkan_missing_falls_back_to_dx12_discrete_first() {
        for swapped in [false, true] {
            let cands = laptop(swapped);
            let order = attempt_order(&cands, &GpuPreference::default(), OsFamily::Windows);
            assert_eq!(
                names(&cands, &order),
                vec![
                    "Dx12:NVIDIA GeForce GTX 1660 Ti",
                    "Dx12:AMD Radeon(TM) RX Vega 10 Graphics",
                    "Gl:NVIDIA GeForce GTX 1660 Ti/PCIe/SSE2",
                    "Dx12:Microsoft Basic Render Driver",
                ],
                "independent of enumeration order (swapped = {swapped})"
            );
        }
    }

    #[test]
    fn user_preferences_reorder_but_keep_fallbacks() {
        let cands = laptop(false);
        let low = GpuPreference {
            power: PowerPreference::LowPower,
            ..GpuPreference::default()
        };
        let order = attempt_order(&cands, &low, OsFamily::Windows);
        assert_eq!(cands[order[0]].name, "AMD Radeon(TM) RX Vega 10 Graphics");
        let named = GpuPreference {
            backend: Some(BackendKind::Gl),
            adapter_name: Some("vega".into()),
            ..GpuPreference::default()
        };
        let order = attempt_order(&cands, &named, OsFamily::Windows);
        assert_eq!(cands[order[0]].name, "AMD Radeon(TM) RX Vega 10 Graphics");
        assert_eq!(
            cands[order[1]].backend,
            BackendKind::Gl,
            "user backend before DX12"
        );
        assert_eq!(order.len(), cands.len(), "nothing is dropped");
    }

    #[test]
    fn with_vulkan_present_vulkan_comes_first_and_software_last() {
        let mut cands = laptop(false);
        cands.push(c(
            BackendKind::Vulkan,
            "AMD Radeon(TM) RX Vega 10 Graphics",
            DeviceKind::Integrated,
        ));
        cands.push(c(BackendKind::Vulkan, "llvmpipe", DeviceKind::Cpu));
        cands.push(c(
            BackendKind::Vulkan,
            "NVIDIA GeForce GTX 1660 Ti",
            DeviceKind::Discrete,
        ));
        let order = attempt_order(&cands, &GpuPreference::default(), OsFamily::Windows);
        let first = &cands[order[0]];
        assert_eq!(
            (first.backend, first.device),
            (BackendKind::Vulkan, DeviceKind::Discrete)
        );
        let tail: Vec<DeviceKind> = order[order.len() - 2..]
            .iter()
            .map(|&i| cands[i].device)
            .collect();
        assert_eq!(tail, vec![DeviceKind::Cpu, DeviceKind::Cpu]);
    }

    #[test]
    fn empty_and_os_orders() {
        assert!(attempt_order(&[], &GpuPreference::default(), OsFamily::Linux).is_empty());
        assert_eq!(backend_order(OsFamily::MacOs), &[BackendKind::Metal]);
        assert!(!backend_order(OsFamily::Linux).contains(&BackendKind::Dx12));
        let json = serde_json::to_string(&GpuPreference {
            backend: Some(BackendKind::Dx12),
            adapter_name: Some("Vega".into()),
            power: PowerPreference::LowPower,
        })
        .expect("serializable");
        assert_eq!(
            json,
            r#"{"backend":"dx12","adapter_name":"Vega","power":"low_power"}"#
        );
    }
}
