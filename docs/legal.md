# Legal restrictions

This is not legal advice. It is a list of rules the team follows until a lawyer's conclusion (task T0-10, human) and questions for the lawyer.

## Development rules (mandatory right now)

1. **No code from the Mojang client or server.** Do not decompile, do not copy logic from decompiled code, do not peek at mappings while writing mechanics. Mechanics are implemented from our own design and public descriptions of behavior.
2. **No Mojang assets in the repository, builds, CI or on our servers.** The player downloads the assets to their own machine after ownership is verified. **Development and testing (owner decision 2026-11-21):** developers may use the Mojang assets of their own licensed installation on their own machine — the asset loader reads them from a local path, and tests that need real assets are opt-in and run locally. They are never committed, uploaded to CI or bundled into builds.
3. **The game runs on our own fallback assets.** A complete own asset set — before the public release.
4. **Name and brand** do not contain "Minecraft" and do not imitate the logo. The formula "Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft" — in the "About" menu and on the website.
5. **Microsoft tokens** are not stored on our servers.
6. **Monetization** — only after consultation (Mojang's server rules restrict selling gameplay advantages).
7. **Offline mode for development (owner decision 2026-11-21).** Client and server may run without Microsoft login: a local player name, no ownership check, no Microsoft or Mojang network calls. It is for development, tests and private test sessions only: off by default, never enabled on a publicly reachable server, and no public release of the client or server until the answers in T0-10 arrive.

**Status 2026-11-21:** the questions below were sent (T0-10). Development does not wait for the answers: offline mode (rule 7) replaces Microsoft login for testing, and developers test with assets from their own licensed installation (rule 2). Nothing is launched publicly until the answers arrive.

**Third-party content (2026-11-22):**
- **Tectonic** (Apollo) is the reference for the extended world generator. Modrinth declares MIT, but the source repository has no LICENSE file, so the copyright notice MIT requires is unknown: ideas and parameters may be ported with credit in `THIRD_PARTY.md`; verbatim copies only after a LICENSE with a copyright line is published or confirmed in writing. Any vanilla Minecraft data inside it is Mojang's and never enters the repository.
- **Terralith** (Stardust Labs) is **not used**: its license forbids copying parts into other projects and any use in AI projects.

**Other engines and servers (2026-11-26, [review](research/external-projects-review-2026-11-26.md)).** Licenses below are as reported, not yet verified: open the LICENSE file before porting anything, and record every port in `THIRD_PARTY.md`.
- **GPL / LGPL projects — ideas only, never code:** Folia, Paper, Purpur, Pumpkin (GPL-3.0), Horizon (LGPL-3.0 after a switch from MIT; its public repository was reportedly removed and it was reportedly in a dispute over code copied from Iris), Iris (LGPL-3.0). Reading them to understand an approach is fine; copying or translating their code is not. Our engine is not GPL, so even a translated function would be a problem.
- **OptiFine** — proprietary: ideas only.
- **Shader packs written for Iris/OptiFine** belong to their authors. They never enter the repository, CI or builds, like any other player pack (ADR-0014).
- **Permissive projects (MIT / Apache-2.0):** mc.rs (`andreypfau/mcrs`), FerrumC — code may be reused with the copyright notice and an entry in `THIRD_PARTY.md`, after checking that the code itself is not derived from decompiled Minecraft (rule 1).

## Questions for the lawyer

- Do the EULA and Usage Guidelines allow the owner of the game to download and locally convert Minecraft assets for a third-party engine?
- Do Microsoft's terms allow passing a Minecraft token to our login service to verify ownership?
- What about monetizing servers on a custom engine if Mojang assets are used locally?
- Is a separate trademark registration needed for the project name?
