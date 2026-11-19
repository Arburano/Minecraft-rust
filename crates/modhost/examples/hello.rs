//! Headless execution of the example component against a local region.

use std::time::Duration;

use modhost::{BlockPos, ModHost, Region};
use world::BlockStateId;

fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: hello <hello_mod.wasm>"))?;
    let host = ModHost::new()?;
    let prepared = host.load(path)?;
    let region = Region::new(
        BlockPos { x: 0, y: 0, z: 0 },
        [("hello:example".into(), BlockStateId(1))],
    );
    let mut worker = prepared.instantiate(region, Duration::from_secs(1))?;
    let declaration = worker.init(Duration::from_secs(1))?;
    println!("{} systems declared", declaration.systems.len());
    worker.run_system(1, Duration::from_secs(1))?;
    println!(
        "{} block changes committed",
        worker.region().section().non_air()
    );
    Ok(())
}
