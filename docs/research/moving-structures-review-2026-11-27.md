# Moving block structures: how Create: Aeronautics and Sable do it (review, 2026-11-27)

The owner wants moving structures "as in Create: Aeronautics" for the industrial build: giant flying constructions with physics and collision. This note records what the public pages say about how that mod family works, what we did not find, and what we take from it. Design: [ADR-0019](../adr/0019-moving-block-structures.md), [moving-structures.md](../architecture/moving-structures.md).

## Projects

- **Create: Aeronautics** (the "Simulated Project", `Creators-of-Aeronautics/Simulated-Project`): the content mod. It adds assembly, redstone components and the tools for "simulated contraptions", and flying with "hot air, propellers, and magic floating rocks".
- **Sable** (`ryanhcode/sable`): the physics library underneath. It "does not add any content to the survival game, or any manner of constructing sub-levels in survival"; it provides the moving structures and the physics.

## What the pages say

### Sub-levels: a moving structure is a small world

> "Sub-levels contain normal Minecraft chunks, entities, and block-entities, but exist at a separate dynamic position and orientation within Minecraft levels." (Sable README)

- A structure's blocks live in a **plot**: ordinary chunks, which are then placed in the world with a position and an orientation. Block entities, entities and all block logic keep working inside, because inside the plot it is an ordinary world.
- Entities spawned inside a plot are "kicked" out: teleported to their global position, with the sub-level's velocity applied. Tags keep some inside (armor stands, paintings), destroy some when they leave, or destroy them with the sub-level.
- Credit for "the sub-level splitting region algorithm" (a structure that breaks into pieces becomes several sub-levels) and for floating blocks.

### Physics: Rapier (Rust), natively

> "Sable contains a physics pipeline (based on the Rapier physics engine written in Rust)" (Modrinth)

Rapier is compiled to a native library and called from Java. Physics data is set per block, in datapacks (`physics_block_properties`):

| Property | Meaning | Default |
| --- | --- | --- |
| `mass` | mass of the block | 1.0 |
| `inertia` | inertia multiplier per axis, times the mass | [1/6, 1/6, 1/6] (a uniform cube) |
| `volume` | volume for buoyancy | 1.0 |
| `restitution` | bounciness | 0.0 |
| `friction` | friction | 1.0 |
| `fragile` | breaks on impact | false |
| `floating_material`, `floating_scale` | the block makes lift ("floating blocks") | — |

Tags give presets: super light 0.25, light 0.5, heavy 2.0, super heavy 4.0; half volume; slippery; bouncy (restitution 0.5).

Physics per dimension (`dimension_physics`):

- `base_gravity`: default `[0, -11, 0]` m/s²;
- `base_pressure`: default 1, **0 for a vacuum**;
- `pressure_function`: pressure by altitude, as Bézier control points;
- `universal_drag`: 0.09 on all motion;
- `magnetic_north`.

So space is "a dimension with zero pressure and its own gravity".

### Entities on a moving structure

> "Entities can be outside of the plot of a sub-level, but still move with the sub-level (ex. A player standing on a sub-level, or a cow in a pen on a sub-level)."

Such entities **track** the sub-level:

- they are "networked relative to the sub-level";
- they are "interpolated relative to the sub-level";
- they move with it as it rotates and translates;
- a player who logs out on a ship logs back in at the same place relative to the ship.

### Lift (third-party add-on, as an example of the gameplay)

The "Create Balloon" add-on (MIT, describes itself as AI-written) shows the kind of rules lift mods use:

- the total lift of all balloons must be at least the structure's mass, or it does not take off;
- lift falls exponentially with altitude, as the air pressure does;
- the gas cools and needs a heat source;
- a gyroscope adds a righting force that keeps the structure level.

## Not found

The pages we read do not describe any of these; we design them ourselves:

- how a plot is stored (a separate world or a region of the same world), the transform representation;
- how collision shapes are built from blocks;
- how mass, center of mass and inertia are summed;
- collision with terrain and between structures;
- the contact between a player's feet and a moving deck;
- the propeller, floating-block and buoyancy formulas;
- the network protocol and tick rate for structures;
- the assembly and disassembly algorithms.

Sable's README calls it "an incredibly intrusive mod" with "extensive compatibility issues" (mixins into Minecraft). We have no such problem: our engine is built for this from the start.

## Licenses

- **Sable: PolyForm Shield 1.0.0** — not an open-source license; it forbids competing uses. **Ideas only, no code**, like the GPL projects in `legal.md`. We do not read its source.
- **Create: Aeronautics (Simulated Project):** license not checked; ideas only until checked.
- **Rapier** (Dimforge): Apache-2.0, a Rust crate (`rapier3d`) we can depend on directly (rule 5: justification in ADR-0019).

## What we take

1. **A moving structure is a small world with a pose** (our "grid"): same sections, same block logic, same meshing, drawn and collided through its transform. This is the key idea, and it fits our engine: `world::Section` storage and the mesher work in local coordinates unchanged.
2. **Rigid-body physics with Rapier**, in Rust, without a native bridge.
3. **Physics properties as block data** (mass, inertia, volume, friction, restitution, fragile, lift), with tags for presets: designers tune ships without code.
4. **Physics per dimension** (gravity, pressure by altitude, drag): space and other planets are data.
5. **Entities ride relative to the structure**: networked, interpolated and saved in its frame.
6. **Splitting:** a structure cut in two becomes two structures.

Sources:

- [Sable README](https://github.com/ryanhcode/sable)
- [Sable on Modrinth](https://modrinth.com/mod/sable)
- [Sable wiki: Block Physics Properties](https://github.com/ryanhcode/sable/wiki/Block-Physics-Properties)
- [Sable wiki: Dimension Physics Data](https://github.com/ryanhcode/sable/wiki/Dimension-Physics-Data)
- [Sable wiki: Working with Entities](https://github.com/ryanhcode/sable/wiki/Working-with-Entities)
- [Simulated Project README](https://github.com/Creators-of-Aeronautics/Simulated-Project)
- [Create Balloon (third-party add-on)](https://modrinth.com/mod/e9CPu9Ub)
- [PC Gamer on Create: Aeronautics](https://www.pcgamer.com/games/survival-crafting/this-hugely-impressive-minecraft-mod-lets-you-build-all-manner-of-functioning-vehicles-including-planes-combine-harvesters-and-even-airships/)
