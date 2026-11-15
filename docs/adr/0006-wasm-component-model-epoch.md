# 0006. WASM Component Model, epoch, host-side state

Status: Accepted (2026-11-15)

## Context

Mods must be safe, written in any language, with hot reload and limits. Test (wasmtime 50, VM): a core-module call 17–30 ns, a component call ≈ 0.4 µs; 1 000 separate component calls — 0.5 ms, a batch — ≈ 3 µs; epoch 0–9%, fuel +50–92% on code with calls. A `Store` cannot be called from multiple threads. An instance per region — 1–15 GB.

## Decision

Level 2 mods are WebAssembly components under the `wit/engine-api.wit` contract; the host is `wasmtime`. Hooks are batched only. Epoch is always enabled; fuel is only used in tests and replays. One mod instance per worker thread (`InstancePre` + pooling allocator). Mod state lives in the host's ECS components. World changes from a call are applied atomically, and only after success.

## Consequences

- Safe third-party mods, any language with WASI 0.2.
- Batched APIs are a required design discipline.
- Component Model tooling is still changing — versions are pinned in the SDK.

## Alternatives considered

- Native DLLs — no ABI and no sandbox.
- Lua/JS — slower, weaker typing, a separate runtime.
- Core modules with a custom ABI — calls 15–20× cheaper, but without WIT types; remains the fallback option.

## References

docs/modding/overview.md; docs/research/optimization-tests.md.
