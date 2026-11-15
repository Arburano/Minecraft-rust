# Simulation, physics, mob AI

Crate: `sim`. Experience priorities — PRD §14: smoothness, Minecraft feel, smart mobs.

## Player physics

- Movement feel like Minecraft: speeds, acceleration, jumping, sliding, swimming, ladders. The numbers live in the `core` datapack (not hardcoded in the engine) so mods can tune them.
- AABB collisions with blocks (axis-wise sweeping), block collision shapes from the registry.
- The same code on the client (prediction) and the server (authority).

## Prediction and reconciliation

- The client sends intents with a tick number; applies them locally right away.
- The server acknowledges the number of the last processed intent and the authoritative state.
- Divergence → the client rolls back to the server state and replays unacknowledged intents. A correction threshold makes small divergences smooth out rather than "teleport".
- Digging and placing blocks are also predicted; a server rejection rolls the block back.

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
