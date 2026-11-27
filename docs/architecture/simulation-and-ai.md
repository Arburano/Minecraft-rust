# Simulation, physics, mob AI

Crate: `sim`. Experience priorities — PRD §14: smoothness, Minecraft feel, smart mobs.

## Player physics

- Movement feel like Minecraft: speeds, acceleration, jumping, sliding, swimming, ladders. The numbers live in the `core` datapack (not hardcoded in the engine) so mods can tune them.
- AABB collisions with blocks (axis-wise sweeping), block collision shapes from the registry.
- The same code on the client (prediction) and the server (authority).

Implementation (T1-12, `sim::physics`, `sim::raycast`):

- `PhysicsConfig` is read from `mods/core/data/player/physics.toml` (unknown keys are rejected): box 0.6 × 1.8, eye 1.62, step 0.6, gravity 0.08 and vertical drag 0.98 per tick, jump 0.42, ground/air friction 0.546/0.91, walk/air acceleration 0.1/0.02, sprint ×1.3, fluid drag/acceleration/gravity/swim-up, reach 4.5, and the swim blocks (`core:water`). This gives walk ≈ 4.3 blocks/s and a jump of ≈ 1.25 blocks.
- `step(body, input, cfg, world)` per 20 TPS tick: acceleration along the view yaw, jump or swim-up, then the move with collisions, then gravity and friction. Collisions sweep the box axis by axis (Y, X, Z) against full cells; step-up tries "up by the step height, across, down" when the horizontal move is blocked on the ground and keeps it if it goes farther. Unloaded cells collide (no falling into unloaded columns). `Collision::Model` counts as a full cube until model shapes exist. `interpolate` gives the render position between two ticks.
- Plain `f64` with a fixed evaluation order: the same inputs from the same state give bit-identical positions (tested over 600 scripted ticks).
- **Flight** (owner request 2026-11-22, the creative-mode feel):
  - a double-tapped jump (two presses within 300 ms; the toggle is a one-tick input event) starts or stops flight; the profile can forbid it (`can_fly`);
  - while flying there is no gravity: jump moves up, descend (Shift) moves down, sprint doubles the horizontal acceleration, the vertical drag is 0.6;
  - at speed ×1 that gives ≈ 11 blocks/s horizontally, ≈ 22 sprinting, ≈ 7.5 vertically;
  - starting flight zeroes the vertical speed, so the player hovers where they double-tapped; touching the ground ends flight;
  - the mouse wheel sets the fly speed multiplier while flying (×1.25 per notch, range `fly_speed_range`, 0.25–8), like spectator mode; while walking the wheel scrolls the hotbar;
  - flight and the multiplier are saved with the player in `world.toml`;
  - all numbers are in `physics.toml`.
- `raycast` is a voxel DDA returning the hit cell, the entered face and the distance; the client's `pick` skips air and swim blocks.
- Block edits on the client: `client::edit` (`Editor::set_block` → loader re-mesh of the affected mesh chunks on the main thread → deferred relight → `BatchSync` re-copy). `place` refuses cells holding a non-replaceable block and colliding blocks that would overlap the player.

## Prediction and reconciliation

- The client sends intents with a tick number; applies them locally right away.
- The server acknowledges the number of the last processed intent and the authoritative state.
- Divergence → the client rolls back to the server state and replays unacknowledged intents. A correction threshold makes small divergences smooth out rather than "teleport".
- Digging and placing blocks are also predicted; a server rejection rolls the block back.

Implementation (T2-03, `client::session`, `server::game`, `server::input`):

- **Hash, not state:** each intent carries a 32-bit hash (FNV-1a over the exact `f64` position, velocity, flags and fly speed) of the client's state after it. The server applies the intent with the same `sim::physics::step` and compares. Agreeing states cost nothing; on a mismatch the server sends a `Correction` (exact state after that tick) in every datagram until a later intent's hash agrees.
- **Rollback and replay:** the client keeps the last 128 predicted ticks. On a correction it restores the server's state at that tick and replays its later intents; a repeated correction it already agrees with is ignored.
- **Smoothing:** the change of the drawn position becomes an offset that decays with a 100 ms time constant; a correction of more than 2 blocks is applied at once (a teleport).
- **Determinism:** both sides convert intents through `server::input` (quantized yaw, wheel steps at the tick). A golden hash of a 600-tick route (walk, sprint, jump, strafe, fly, wheel, descend) pins the result; debug and release builds agree (`crates/server/tests/determinism.rs`).
- **Other players** arrive as entity deltas (networking-protocol.md); the client glides each one from its drawn position to its newest one over the time between its last two updates (50–300 ms; > 8 blocks is a teleport), and draws them as boxes (`render::entities`) until models exist (stage 4).

## Entities and ECS

- Components: `NetId`, `Transform`, `Velocity`, `Collider`, `Health`, `Ai*`, etc. Mod components are stored by the host (`components` in WIT).
- Block entities (chests, furnaces) are a separate map in the column, ticked only if they have tick behavior.

## Mob AI (not "dumb mobs")

- Full AI within a radius of ≈ 48 blocks of the nearest player (the number is tuned by playtesting).
- Beyond that, the AI decision frequency falls with distance (simulation LOD): every 2nd, 4th, 8th tick. Physics and animation remain, so the mob does not visibly "freeze".
- Pathfinding is asynchronous in the task pool (`Visible` priority near the player), and the result is applied in the next tick. A per-region task limit per tick.
- Spawning only within the radius where a player can see the mob (as in Minecraft: no closer than ~24 blocks, no farther than the simulation distance), by zone tables. Despawning of distant ones — as in Minecraft.
- We do not do: spawning mobs "in the air with AI turned off" (PRD §14, No. 4).

## Simulation LOD

| Distance from player | Entities | Block mechanics |
| --- | --- | --- |
| ≤ simulation distance (6–8 chunks on the server) | full tick; AI falls with distance | 20 TPS |
| farther | not ticked, stored | not ticked |

Block mechanics (farms, contraptions) within the simulation distance are never slowed down — otherwise farms break (PRD §14, No. 5). A mod can mark a system "every tick".

## Global state

Time of day, weather, tables — in the global region. Regions read its snapshot from the previous tick; they change it with commands in `Exchange`.
