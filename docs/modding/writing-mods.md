# How to write mods

This guide is for mod authors. Status: the SDK and `engine-cli` are still being built (stage 3); the examples show the target API.

## 1. Start with data

Most content is a datapack: blocks, items, recipes, loot, biomes, zones. Code is needed only for behavior that data cannot describe. See [datapacks.md](datapacks.md).

## 2. Create a mod

```bash
engine-cli new copper_pipes      # template: mod.toml, data/, assets/, src/lib.rs
cd copper_pipes
engine-cli dev                   # local server + client, hot reload of data, assets and .wasm
```

## 3. Behavior in code (Rust → wasm32-wasip2)

```rust
use engine_mod_sdk::prelude::*;

#[derive(Component, Default)]
#[replicate]                      // only for side = "both"
struct Flow { amount: u16 }

#[mod_init]
fn init(app: &mut ModApp) {
    app.component::<Flow>("copper_pipes:flow")
       .system("flow", Phase::Simulate, flow_system)   // access is derived from the signature
       .every_n_ticks(2)
       .on::<BlockPlaced>(on_placed);
}

fn flow_system(region: &mut Region, pipes: Query<(&BlockPos, &mut Flow)>) {
    let mut changes = Vec::new();
    for (pos, mut flow) in pipes {
        let below = pos.down();
        if region.contains(below) {
            changes.push((below, flow.amount / 2));
        } else {
            region.send_to(below, "copper_pipes:transfer", &(flow.amount / 2));
        }
        flow.amount /= 2;
    }
    region.apply_flow_batch(&changes);    // one host call per system
}
```

What matters here:

- State (`Flow`) lives in the host, not in the mod's static variables.
- A neighboring region only through `send_to`; the response arrives on the next tick.
- Changes come in a batch.

## 4. Tests

```bash
engine-cli test       # headless server: fixed seed, N ticks, state checks; fuel instead of epoch → repeatable
engine-cli bench      # mod time per tick and memory; fails if it exceeds [budget]
```

## 5. Publishing

```bash
engine-cli publish    # check, build, ed25519 signing, upload to the registry
```

## 6. Performance debugging

- `/profile mods` on the server — each mod's time per tick by region.
- Rules — [performance-rules.md](performance-rules.md).

## Other languages

Any language with WASI 0.2 Component Model support (C, Zig, Go via TinyGo, AssemblyScript) against the same WIT. The official SDK is Rust; the second language is stage 7.
