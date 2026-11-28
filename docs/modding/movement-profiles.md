# Movement profiles and abilities

Decision: [ADR-0013](../adr/0013-replaceable-movement-and-worldgen.md) (gameplay slots). Code: `sim::physics` (the step), `sim::abilities` (the primitives). Task: T1-18.

A build's movement comes from the one mod that fills the `movement` slot (`provides = ["movement"]` in its `mod.toml`). That mod ships `data/movement/profile.toml`: the base numbers and a list of abilities. A build with a different feel (parkour, RPG classes, low gravity) lists a different movement mod; the engine has no built-in moves.

Shipped providers:

| Mod | Feel | Used by |
| --- | --- | --- |
| `movement` | the classic feel: walk, sprint, jump, swim, fly (double-tap jump) | `classic`, `classic_plus` |
| `movement_agile` | the classic numbers plus dash, double jump, wall slide, crawl, glide; no flight | `classic_agile` (example; the feel needs the owner's review) |

## Base numbers

Per 20 TPS tick, in blocks: box (`width`, `height`, `eye_height`), `step_height`, `gravity`, `vertical_drag`, `jump_velocity`, `sprint_jump_boost`, ground and air friction and acceleration, `sprint_multiplier`, fluid drag, acceleration and gravity, `swim_up`, `reach`, `swim_blocks`, and flight (`can_fly`, `fly_accel`, `fly_vertical_accel`, `fly_vertical_drag`, `fly_sprint_multiplier`, `fly_speed_range`). Unknown keys are rejected. The classic values and what they give are in `simulation-and-ai.md`, "Player physics".

## Abilities

```toml
[[abilities]]
id = "dash"                       # unique in the profile
trigger = "sprint_pressed"        # when it may start
conditions = ["moving", "not_flying"]   # all must hold
cooldown = 20                     # ticks after an active tick (default 0)
charges = 0                       # uses until landing or entering a fluid; 0 = unlimited
animation = "dash"                # animation state while active (entity animations come later)
effects = [{ kind = "impulse", forward = 0.9, up = 0.05 }]
```

An ability is **active** on a tick when its trigger holds, all its conditions hold, its cooldown is over and it has a charge left. Every active tick applies all its effects, starts its cooldown and uses a charge. Up to 8 abilities per profile.

| Trigger | Holds when |
| --- | --- |
| `jump_pressed`, `sprint_pressed`, `descend_pressed` | the key went down this tick |
| `jump_held`, `sprint_held`, `descend_held` | the key is held |

| Condition | Holds when |
| --- | --- |
| `on_ground` | standing on something (after the last tick) |
| `in_air` | not on the ground, not in a fluid, not flying |
| `in_fluid` | inside a swim block |
| `touching_wall` | a wall within 0.05 blocks on a horizontal side |
| `moving` | movement keys are pressed |
| `falling` | moving down |
| `not_flying` | not flying |

| Effect (`kind`) | Fields | What it does |
| --- | --- | --- |
| `impulse` | `forward`, `up`, `right` | adds velocity in the view frame (blocks per tick) |
| `set_velocity` | `vertical`, `forward` (both optional) | sets the vertical velocity and/or the horizontal speed along the view (sideways velocity cleared) |
| `hold_surface` | `slide`, `climb` | falling is limited to `slide`; with `climb > 0`, moves up at that speed (climbing) |
| `box_size` | `height`, `eye_height` | a different collision box while active (crouch, crawl); the normal box returns only when there is room |
| `gravity_scale` | `scale` | multiplies gravity this tick (glide, low-gravity jump) |

Typical patterns:

- **instant moves** (dash, double jump, blink): a `*_pressed` trigger with `impulse` or `set_velocity`, plus a `cooldown` or `charges`;
- **held states** (glide, wall slide, crawl, climb): a `*_held` trigger with `gravity_scale`, `hold_surface` or `box_size`.

The ability step runs after the normal intent (walking, jumping, swimming, flying) and before the movement with collisions.

Notes:

- A double-tapped jump also toggles flight when `can_fly = true`; a profile with a double jump should set `can_fly = false`.
- The order of abilities matters only when two are active on the same tick: effects apply in file order.
- Not yet: abilities written as WASM code for moves the primitives cannot express (stage 3; ADR-0013, decision 3), and a separate key for abilities.

## Determinism and network

- Ability state (cooldowns, charges, the active box, last tick's buttons, active bits) is part of the player body. The client predicts it, the server simulates it, and a correction carries it (`Correction::abilities`, protocol version 4).
- Profiles without abilities run exactly the code they ran before abilities existed: a golden trace test pins the classic movement bit for bit, and their state hash is unchanged.
- Test: `client/tests/remote.rs` (`abilities_predict_without_corrections`) plays dash, double jump, glide and crawl against a real server with zero corrections.

## Slots and worlds

- One provider per slot: a build with two movement mods fails at load with both names; a build without one fails with "build X has no mod that provides \"movement\"".
- `server.toml` records the provider of every slot (`mod@version`). A different movement provider may open a world, because nothing in the world depends on it; the server prints a note. A different world generator is refused (`worldgen.md`, "Engines").
