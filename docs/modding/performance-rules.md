# Rules for fast mods

1. Everything that can be described by data, describe by data.
2. No global state in a mod: data lives in host components and resources.
3. One host call per system, not per block or entity. Batches: `get-blocks`, `set-blocks`, `components.get/set` with lists.
4. Rare work — `every_n_ticks(n)`, not a check every tick.
5. Heavy computation (pathfinding, generation) — through host tasks; the result comes back as an event.
6. To a neighboring region — only `send-to`; do not build logic that waits for a response within the same tick.
7. Do not keep large buffers in linear memory between calls — it is scratch.
8. Client code does not make gameplay decisions: the server decides.
9. Do not rely on event order between different regions; within a region the order is stable.
10. Systems that must tick every tick even far from a player (rarely!) are marked explicitly; otherwise the engine may lower their frequency outside the simulation distance.

Check: `engine-cli bench` fails if the mod exceeds `[budget].tick_ms`; in production the server warns, then disables the mod.
