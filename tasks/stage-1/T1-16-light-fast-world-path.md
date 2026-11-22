# T1-16. Faster world-path lighting

- **Status:** todo (after the main stage 1 tasks; owner decision 2026-11-22)
- **Depends on:** T1-09
- **Owner:** agent (cloud) + laptop measurement

## Goal

T1-09 lights edits correctly but through a costly world path: rebuilding a 48³ snapshot per processed section (VM p95 ≈ 0.7–1.2 ms per torch edit, ≈ 13–15 ms for an edit that changes direct sky). For stage 1 the owner accepted this (option 1: updates are deferred and capped per tick; the ≤ 100 / 200 µs budget applies to the BFS). This task replaces the world path with a faster algorithm **without changing what the player sees or feels**.

## Constraints (owner)

- Light must look the same: the final light after any sequence of edits stays bit-for-bit equal to a fresh full lighting (the T1-09 tests stay as they are).
- No new visible artifacts: no seams, no flicker, no longer delay before light appears after an edit than in T1-09.
- Do not rewrite unrelated systems for this; if a candidate needs an architecture change (e.g. persistent light storage), write an ADR with status "Proposed" first.

## Candidates (from the T1-09 report)

1. A smaller window around the edit (32³ instead of the section-aligned 48³).
2. Persistent per-region light/opacity grids with the BFS working in place (no snapshot copies) — likely the way to ≤ 100 µs; needs an ADR.
3. Incremental vertical propagation of direct sky instead of recomputing whole levels below.
4. Running `LightUpdates::process` as a background job with copied inputs.

## Acceptance criteria

- [ ] World-path edit p95 measured before/after on the laptop; target ≤ 100 µs (add) / ≤ 200 µs (remove) for non-sky edits, or the best reachable number with the owner's agreement.
- [ ] Sky-changing edits measured; no frame spike from lighting in the T1-15 walk.
- [ ] All T1-09 equality tests unchanged and green; a visual check on the laptop (before/after screenshots of the same scene) shows no difference.
