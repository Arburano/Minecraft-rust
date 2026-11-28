# Zones

Named world areas with rules: who may build, who may walk in, whether players may fight, and what spawns there (PRD §14 No. 11). Zones are the engine capability `zones` ([ADR-0018](../adr/0018-engine-capabilities-per-build.md)): a build gets them only if one of its mods requires them.

```toml
# mod.toml
[requires]
capabilities = ["zones"]
```

A build without `zones` has no zone index, no zone checks and no zone file; the server's hot paths check one `None`.

A zone is a static map layout. It is not a threading region ([glossary](../glossary.md)): regions follow the players, zones stay where they are drawn.

## Format

`<mod>/data/zones/<name>.toml`, one zone per file:

```toml
id = "rpg_world:old_town"         # namespace = the mod id
name = "Old town"                 # display name (optional)
priority = 10                     # where zones overlap, the higher decides (default 0)
difficulty = 1                    # for the build's content: spawning, loot (default 0)
areas = [                         # boxes, every range inclusive; y is the whole height when absent
  { x = [-64, 63], z = [-64, 63] },
  { x = [64, 95], z = [-8, 7], y = [60, 90] },
]
members = ["alice"]               # player names exempt from a denied build or entry rule
rules = { build = false, pvp = false }   # build, entry, pvp; absent = left to the zones below
spawns = [                        # absent = left to the zones below; [] = nothing spawns here
  { entity = "mobs_rpg:guard", weight = 80, group = [1, 2] },
  { entity = "mobs_rpg:cat", weight = 20, light = [8, 15] },
]
```

- `id` must be `namespace:name` (`a-z`, `0-9`, `_`). In mod data the namespace is the mod id.
- A later mod replaces a zone of a mod it depends on with `override = true`; any other repeated id is a load error naming both mods.
- Unknown fields are errors. Spawn weights must be > 0, groups a range `1 ≤ min ≤ max`, light a range in `0..=15`.
- At most 4,096 zones per world and 256 areas per zone.

## How rules are resolved

At a block, the zones that contain it are taken in decision order: higher `priority` first, equal priorities by id. Each rule is resolved on its own: the first zone that sets it decides. Where no zone sets a rule, it is allowed.

Example: a no-build reserve (priority 0) holds a town (priority 5) that sets only `pvp = false`, and the town holds a plot (priority 9) with `build = true`. In the plot, building is allowed (the plot) and PvP is not (the town). In the rest of the town, building is denied (the reserve).

- **build:** breaking and placing blocks. A member of the deciding zone may build anyway.
- **entry:** a player may not step from a cell where entry is allowed into one where it is denied. A player who is already inside (the zone was drawn around them, or they are a member who lost membership) may move and leave. A member may enter. The check uses the cell of the feet position, so the body may overlap the border by half its width.
- **pvp:** one player may hurt another only if PvP is allowed at both positions. Members do not bypass it.
- **spawns:** the spawn system takes the table of the deciding zone and does not look at other lists. Picking is by weight among the entries whose light range holds the light at the spawn position. Where no zone sets a table, the build's default spawning applies.

Entities other than players cross zone borders freely. Only spawning is restricted by zones, so a mob that walks into a town behaves as anywhere else.

## Zones stored in the world

Besides the mods' zones, a world has its own: `<world>/zones/zones.toml`, a list of `[[zone]]` tables in the same format. A world zone with the id of a mod zone replaces it, so an operator can change a mod's zone for one world. Removing the world zone brings the mod's zone back.

The server API (`server::zones::ZoneControl`, from `Server::zones()`; mods get it with the mod API, stage 3):

- `define(zone)` adds or replaces a world zone. The definition is checked when it is called.
- `remove(id)` removes a world zone.

Changes take effect at the end of the next world tick, never in the middle of one, and are saved to the world file right away. A change that would break the index (for example, more than 4,096 zones) is refused and logged.

A world opened by a build with `zones` records the capability in `server.toml`. After that, a build without `zones` refuses the world ([builds.md](builds.md), "Engine capabilities").

## Cost

The index cuts the map into 64 × 64-block tiles. Each tile lists the areas that overlap it, in decision order. Areas wider than 65,536 tiles (whole-map zones) go to a short list that every lookup checks. A lookup is one hash of the tile and a scan of its list, stopping once every rule is decided.

| Zones | Lookup per entity (`resolve`) | Every area scanned (reference) |
| --- | --- | --- |
| 0 | 7 ns | 9 ns |
| 10 + 2 whole-map | 15 ns | 13 ns |
| 100 + 2 | 15 ns | 100 ns |
| 1,000 + 2 | 20 ns | 2,197 ns |
| 4,000 + 2 | 36 ns | — |

These numbers are from a cloud VM (4 vCPU Xeon 2.3 GHz, `cargo bench -p world --bench zones`, 2 runs; the second run is shown, the first was within 8%). Zones are random boxes of 16–256 blocks over 8,192 × 8,192 blocks, and the lookups are 1,000 random positions. The server makes one lookup for every player step and every block edit.

## Not done yet

- **Combat and natural spawning** do not exist yet. `ZoneIndex::pvp_allowed` and `ZoneIndex::spawn_table` are ready for them, and the systems must call them.
- **The client does not know zones.** It predicts an edit in a no-build zone and rolls it back when the server refuses (one round trip). It also learns about a closed border from the movement correction. Sending zone names and rules to the client (a HUD, no mispredictions) is a follow-up.
- **Zones by biome or by generator region** (the `map` engine, `rpg_world`) need the biome of a position at runtime. For now, zones are boxes.
- **Permissions per role** (`roles`): for now, members are a list of player names per zone.
