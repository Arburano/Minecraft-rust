# Open questions

Questions that cannot be decided without the product owner (or a lawyer). An agent does not decide them itself: it uses the default from the "For now" column and marks the place in the code `// OPEN-QUESTION: <number>`.

| No. | Question | For now | Who decides | When |
| --- | --- | --- | --- | --- |
| 1 | Is a web client (WebGL2) needed? | No. Path C (GL ES) is minimal, only for old desktop GPUs | owner | before the end of stage 1 |
| 2 | Are bit-exact server replays needed? | No. Repeatability only with the same region partitioning; fuel only in tests | owner | before stage 4 |
| 3 | Source of truth for entity ownership between shards: JetStream or PostgreSQL with a record version? | Not implemented until stage 6 | architect | stage 6 |
| 4 | Can the Minecraft token be passed to our login service? | Implement per the scheme in auth-and-assets.md, but do not launch publicly | lawyer | stage 0 |
| 5 | Mojang assets in a custom engine: download and conversion on the player's machine | Prepare the code, the game and CI run on fallback assets | lawyer | stage 0 |
| 6 | Capacity of one region (players at tick ≤ 25 ms) | Unknown | benchmark | stage 2 |
| 7 | Real compressed column size and server RAM for 500 players | Estimates from PRD §8 | benchmark on a real world | stage 1–2 |
| 8 | Re-copy a render batch entirely or 8 sub-ranges | Re-copy entirely | benchmark T0-05 | stage 0 |
| 9 | Project name and trademark | Working name of the repository | owner + lawyer | before the open beta |
| 10 | Hiding other players' builds in special modes (creative plots) | Not done in the shared world | owner | stage 4 |
