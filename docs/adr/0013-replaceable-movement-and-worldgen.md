# 0013. Replaceable movement and world generation (gameplay slots)

Status: Accepted (owner, 2026-11-22)

## Context

The owner wants packs that change how the game plays without touching the engine:

- **Movement:** speeds, jumps and other numbers, plus new abilities with their animations (climbing walls, rolls, dashes and the like). One file, or a small mod, replaces the normal movement. Packs that use different movement do not carry the normal one.
- **World generation:** a pack replaces the generator, and the world starts generating differently. The normal generator is not shipped when unused.

Where things stand today:

- **Movement:** T1-12 put movement in `sim::physics` (engine code). Its numbers come from `mods/core/data/player/physics.toml`, but the set of moves (walk, sprint, jump, swim) is fixed in Rust.
- **Generation:** T1-03 put the overworld stages in the `worldgen` crate (engine code), configured by `mods/core/data/worldgen/overworld.toml`. `Pipeline::replace` and `insert_after` exist; mods will reach them through the mod API (stage 3).
- **Rules:**
  - AGENTS.md rule 2: the engine does not know specific blocks and mobs; gameplay lives in mods.
  - ADR-0006: mods are WASM components with batched calls.
  - ADR-0010: the launcher/updater installs what a pack lists.

## Decision

1. **Gameplay slots.** A slot is a part of the game that exactly one provider fills per world. It is chosen when the world is created (or in the server config) and stored in the world metadata (`world.redb`). The first two slots are `movement` and `worldgen` (per dimension). A provider is a mod that declares `provides = ["movement"]` or `provides = ["worldgen:overworld"]` in `mod.toml`. Two providers for one slot is a load error, with a clear message.
2. **The engine keeps only mechanisms:**
   - collision sweep, step-up, raycast, fixed-tick integration and interpolation;
   - the generation pipeline runner, noise primitives and packing;
   - light.
   
   It contains no move set and no generator.
3. **A movement provider** has three parts:
   - **A profile file** `movement/profile.toml`: the base numbers (what `physics.toml` holds today), then a list of abilities. Each ability has an id, an input trigger, conditions (on ground, touching a wall, in fluid, cooldown), the engine primitive it uses with parameters, and the animation state it plays.
   - **Engine primitives** that abilities are built from: impulse, set velocity, hold against a surface, change box size, gravity scale, invulnerability frames. They are small and generic, and they are what makes most abilities "just a file".
   - **Optional WASM code** (`side = both`, deterministic) for an ability the primitives cannot express. It is called once per tick for all players in a batch.
   
   Animations, models and sounds named by the profile ship with the provider mod (client side). The current walk/sprint/jump/swim becomes the profile of a `core-movement` provider.
4. **A world-generation provider** is a generator config (the `overworld.toml` kind) plus its stages. The current stages become the `core-worldgen` provider. Until the WASM mod API exists (stage 3), they stay native behind a cargo feature `builtin-overworld`, so a build for a pack with a different generator leaves them out. Heavy math stays native through batched host functions (noise over a grid), so a WASM generator can still meet the ≤ 1 ms per column target.
5. **Shipping only what is used.** A pack lists its mods; the launcher/updater (ADR-0010) and the server download and load only those. A pack with its own movement or generator does not list `core-movement` / `core-worldgen`, so neither their files nor their code are installed. The engine binary carries no gameplay to leave out, apart from the `builtin-overworld` feature until stage 3.

## Consequences

- **Simpler:** total-conversion packs such as a parkour pack, a different world, or a different feel, without engine changes; the T1-17 extended generator becomes just another `worldgen` provider.
- **Harder:**
  - movement must stay deterministic on client and server across providers;
  - the ability primitives need careful design so most abilities need no code;
  - two more crates or mods to keep in sync with the engine version;
  - prediction and reconciliation (stage 2) must treat ability state as part of the player state.
- **Tasks after approval:**
  - (a) split `mods/core` into `core` (blocks, items), `core-movement` and `core-worldgen`, and add `provides` and slot checks to the manifest;
  - (b) a movement profile format with abilities, and the first primitives, refactoring `sim::physics` to run a profile;
  - (c) the `builtin-overworld` feature;
  - (d) storing slot choices in the world metadata;
  - (e) WIT for movement abilities and generation stages with the mod API (stage 3).
  
  Animations need the entity animation system (later stage).

## Alternatives considered

- **Keep movement and generation in the engine with config files only.** Numbers can change, but new abilities and generators cannot, and unused code always ships.
- **Every ability as WASM code, no primitives.** Most packs would need programming for simple moves, and there would be a component call per ability per player per tick.
- **Native plugins (level 3) for providers.** No sandbox and no stable ABI, so third-party packs are impossible (ADR-0006).
