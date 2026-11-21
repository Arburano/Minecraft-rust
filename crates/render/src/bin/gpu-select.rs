//! Runs the client's backend/adapter selection (T1-05) and prints the result
//! as JSON: the attempt order, failed attempts and the chosen adapter. Exits
//! cleanly after dropping the device, so a crash at exit (seen with GL on the
//! Vega 10 in T0-14) shows up as a non-zero exit status.
//!
//! `cargo run --release -p render --bin gpu-select -- [--backend gl] [--adapter vega] [--low-power]`

use anyhow::{Result, bail};
use clap::Parser;
use render::selection::{BackendKind, GpuPreference, PowerPreference, select_device};

#[derive(Parser)]
struct Args {
    /// Backend to try first: vulkan, dx12, metal, gl.
    #[arg(long, value_parser = parse_backend)]
    backend: Option<BackendKind>,
    /// Try adapters whose name contains this text first.
    #[arg(long)]
    adapter: Option<String>,
    /// Prefer the integrated GPU.
    #[arg(long)]
    low_power: bool,
}

fn parse_backend(s: &str) -> Result<BackendKind, String> {
    serde_json::from_value(serde_json::Value::String(s.to_lowercase()))
        .map_err(|_| format!("unknown backend {s:?}; use vulkan, dx12, metal or gl"))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let preference = GpuPreference {
        backend: args.backend,
        adapter_name: args.adapter,
        power: if args.low_power {
            PowerPreference::LowPower
        } else {
            PowerPreference::HighPerformance
        },
    };
    match select_device(&preference, &wgpu::DeviceDescriptor::default()) {
        Ok(selected) => {
            let failed: Vec<_> = selected
                .failed
                .iter()
                .map(|f| serde_json::json!({ "adapter": format!("{:?}", f.candidate), "error": f.error }))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "chosen": {
                        "backend": selected.candidate.backend,
                        "name": selected.candidate.name,
                        "device": format!("{:?}", selected.candidate.device),
                        "path": selected.candidate.path,
                    },
                    "failed_before": failed,
                }))?
            );
            drop(selected);
            Ok(())
        }
        Err(failed) => {
            for f in &failed {
                eprintln!("failed: {:?}: {}", f.candidate, f.error);
            }
            bail!("no usable GPU adapter ({} attempts)", failed.len())
        }
    }
}
