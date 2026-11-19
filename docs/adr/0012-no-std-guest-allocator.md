# 0012. Isolated canonical-ABI allocator for the no_std example guest

Status: Accepted (2026-11-19; repository owner explicitly approved the unsafe exception)

## Context

The Rust `wasm32-wasip2` standard library adds WASI clock and I/O imports, while the engine contract exposes neither global time nor ambient I/O. The `no_std` guest avoids these imports, but wit-bindgen 0.63 expects this target's standard library to supply `cabi_realloc`; without it, component linking fails.

## Decision

Keep the example guest and SDK `no_std`. Use dlmalloc's global allocator and provide the canonical-ABI export in `mods/examples/hello/src/allocator.rs`. This module alone may use unsafe Rust; the rest of the workspace retains `unsafe_code = deny`.

The ABI caller must supply a live allocation with the stated old size and alignment, or an old size of zero. Layouts are checked before allocation; zero-length results return an aligned sentinel, and allocation failure traps. Allocation and deallocation use the same guest allocator. The symbol is defined exactly once and lives entirely inside sandboxed guest linear memory; it cannot access host pointers.

## Consequences

The component imports only `engine:api` interfaces. No clock, filesystem, network or environment capability is linked. Integration tests exercise canonical allocation through returned declarations, string events and block batches. Unsafe is not permitted elsewhere by this decision.

## Alternatives considered

- Link the Rust standard library and WASI: introduces capabilities not needed by this spike.
- Use a different WASM target: violates T0-07's `wasm32-wasip2` requirement.
- Hand-write a host allocator: would expose the host to a larger unsafe boundary and is unnecessary.
