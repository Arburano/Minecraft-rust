# T1-21. Render reference scene and screenshot tests

- **Status:** todo
- **Depends on:** T1-11, T1-19
- **Owner:** agent (cloud) + owner's laptop for the visual check

## Goal

A fixed, generated test scene and golden-image tests, so that rendering changes are checked by what they look like and not only by green unit tests ([research](../../docs/research/external-projects-review-2026-11-26.md), section 4: "correct code that looks like grey plastic"). It is the prerequisite for the render graph (ADR-0015, T1-22) and for any new geometry bucket (ADR-0016).

## Deliverables

- A reference scene built in code (no world file, no Mojang assets): a flat floor, a "white furnace" room (white walls lit evenly — any visible seam or tint is a bug), a material stand (one block of each fallback texture, transparent and cutout blocks, water), a light-level ramp 0–15, a sky-light shaft, a mesh chunk border and a render batch border in view.
- Fixed camera positions (at least 6, including one under water when water exists) and a fixed time of day.
- An offscreen renderer entry point (extend `crates/client/examples/screenshot.rs` or a test helper) that renders each camera on software Vulkan in CI.
- Golden images under `crates/render/tests/golden/` and a comparison with a per-pixel tolerance and a maximum share of differing pixels; on failure the test writes the actual image and a diff image as CI artifacts.
- A documented way to refresh golden images on purpose (`UPDATE_GOLDEN=1`), never automatically.
- A short guide in `docs/process/testing-and-benchmarks.md`: how to capture the scene in RenderDoc on the laptop.

## Acceptance criteria

- [ ] The scene renders identically on B, Bi and C within the tolerance (software Vulkan).
- [ ] The example shader pack (`docs/examples/shaderpacks/warm-glow`) has its own golden images.
- [ ] A deliberate one-line change in the terrain shader makes the test fail with a readable diff.
- [ ] The owner looked at the scene on the laptop (Vega 10 and GTX 1660 Ti) and confirmed it matches the golden images.

## Out of scope

Water waves, shadows, PBR (they arrive with T1-22 and add their own cameras).
