# Legal restrictions

This is not legal advice. It is a list of rules the team follows until a lawyer's conclusion (task T0-10, human) and questions for the lawyer.

## Development rules (mandatory right now)

1. **No code from the Mojang client or server.** Do not decompile, do not copy logic from decompiled code, do not peek at mappings while writing mechanics. Mechanics are implemented from our own design and public descriptions of behavior.
2. **No Mojang assets in the repository, builds, CI or on our servers.** The player downloads the assets to their own machine after ownership is verified.
3. **The game runs on our own fallback assets.** A complete own asset set — before the public release.
4. **Name and brand** do not contain "Minecraft" and do not imitate the logo. The formula "Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft" — in the "About" menu and on the website.
5. **Microsoft tokens** are not stored on our servers.
6. **Monetization** — only after consultation (Mojang's server rules restrict selling gameplay advantages).

**Status 2026-11-21:** the questions below were sent (T0-10). Until the answer, development assumes "yes" for open questions 4 and 5, **within** the rules above: real assets only on a developer's or player's own machine from their own licensed copy, never in the repository, committed tests, CI, builds or on our servers; no public launch.

## Questions for the lawyer

- Do the EULA and Usage Guidelines allow the owner of the game to download and locally convert Minecraft assets for a third-party engine?
- Do Microsoft's terms allow passing a Minecraft token to our login service to verify ownership?
- What about monetizing servers on a custom engine if Mojang assets are used locally?
- Is a separate trademark registration needed for the project name?
