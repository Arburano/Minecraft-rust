# Login and assets

Crates: `auth`, `assets`. Legal caveats: [../legal.md](../legal.md).

## Microsoft login

Chain: Microsoft OAuth (Device Code Flow or Auth Code + PKCE) → Xbox Live → XSTS → Minecraft token → profile and game ownership check.

Prerequisite: a registered Azure application **approved by Microsoft** via the Minecraft API access form. Without approval `api.minecraftservices.com` returns 403. Task T0-10 (human task, not for an agent).

```text
client ── Minecraft token ──► our login service
                                ├─ checks profile and ownership via api.minecraftservices.com
                                ├─ does not store the Minecraft token
                                └─ issues a session JWT (Ed25519, 15–60 min) + refresh
game servers/gateway verify only our JWT (public key)
```

- Microsoft tokens never reach game servers.
- The refresh token lives in the OS secret store (`keyring`: Keychain, Credential Manager, libsecret). Never in plain files.
- A short-lived JWT is needed so that bans and session theft take effect quickly.
- Fallback login (own accounts) is for the case of a Microsoft outage (PRD §10 risk). Not in stage 1.

### Offline mode (development and tests)

Owner decision 2026-11-21 (legal.md rule 7): development does not wait for Microsoft API approval (T0-10).

- The client starts with a local player name (`--offline-name <NAME>`, 3–16 characters `A-Z a-z 0-9 _`); no Microsoft, Xbox or Mojang calls — the `auth` crate has no network dependency. The player id is derived deterministically from the name: SHA-256 of `"offline-player:" + lowercase name`, first 16 bytes, UUID version 8 (case-insensitive, so saves and tests are repeatable).
- The server admits offline identities only when started with `--offline` (`auth::AdmissionPolicy`); the default is online. In offline mode it listens on `127.0.0.1` by default, accepts `--bind` only for loopback or private LAN addresses (RFC 1918, IPv6 ULA/link-local) and refuses `0.0.0.0`/`::` and public addresses; it logs a warning on start. Default port 47777.
- Offline mode never issues our session JWT for a public gateway; it is a separate, clearly marked code path that can be removed or disabled in release builds.
- All automated tests and CI use offline mode or the mock login server.
- Reference: [wiki.vg MSA](https://www.wiki.vg/MSA), Rust example: `lighty-auth`.

## Assets

- Source: the official Mojang version manifest and asset index. They are downloaded **on the player's machine** after the ownership check. We do not distribute them or put them in the repository.
- Conversion (local): textures → texture arrays with mipmaps; models → baked templates; sounds → the audio engine format. The result goes to `<data>/cache/assets/<version>/`.
- Cache key: hash of the source file + converter version.
- **Fallback assets** (`core:fallback/*`): our own minimal textures, models, sounds. The game and all tests must work without Mojang assets. CI never downloads Mojang assets.
- **Local installation for development** (owner decision 2026-11-21): the loader can be pointed at a developer's own licensed Minecraft installation (config or environment variable, e.g. `ASSETS_LOCAL_INSTALL=<path to .minecraft>`), reading the client jar and the asset index directly, without the ownership check. Converted output goes to the normal local cache. Tests with real assets are opt-in: they skip with a message when the variable is not set, so CI stays on fallback assets.
- Lazy loading: at startup only what is needed for the menu and nearest blocks; mob and zone models/sounds on approach.

## Tests

- The full login cycle is a manual test (needs a real account); in CI, a mock server with the same responses.
- Conversion: golden tests on fallback assets.
- The game starts even if there are no Mojang assets at all.
