//! Dumps all Vulkan, DX12 and GL adapters as JSON without requesting devices.

use anyhow::Result;
use render::AdapterReport;
use serde::Serialize;
use std::io::{self, Write};

#[derive(Serialize)]
struct ProbeReport {
    schema_version: u32,
    os: &'static str,
    architecture: &'static str,
    attempted_backends: [&'static str; 3],
    adapters: Vec<AdapterReport>,
}

fn main() -> Result<()> {
    let backends = [
        wgpu::Backends::VULKAN,
        wgpu::Backends::DX12,
        wgpu::Backends::GL,
    ];
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: backends
            .into_iter()
            .fold(wgpu::Backends::empty(), |a, b| a | b),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let mut adapters = Vec::new();
    for backend in backends {
        for (index, adapter) in pollster::block_on(instance.enumerate_adapters(backend))
            .iter()
            .enumerate()
        {
            adapters.push(AdapterReport::from_adapter(adapter, index));
        }
    }
    let report = ProbeReport {
        schema_version: 1,
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        attempted_backends: ["Vulkan", "Dx12", "Gl"],
        adapters,
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)?;
    Ok(())
}
