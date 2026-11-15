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
- Reference: [wiki.vg MSA](https://www.wiki.vg/MSA), Rust example: `lighty-auth`.

## Assets

- Source: the official Mojang version manifest and asset index. They are downloaded **on the player's machine** after the ownership check. We do not distribute them or put them in the repository.
- Conversion (local): textures → texture arrays with mipmaps; models → baked templates; sounds → the audio engine format. The result goes to `<data>/cache/assets/<version>/`.
- Cache key: hash of the source file + converter version.
- **Fallback assets** (`core:fallback/*`): our own minimal textures, models, sounds. The game and all tests must work without Mojang assets. CI never downloads Mojang assets.
- Lazy loading: at startup only what is needed for the menu and nearest blocks; mob and zone models/sounds on approach.

## Tests

- The full login cycle is a manual test (needs a real account); in CI, a mock server with the same responses.
- Conversion: golden tests on fallback assets.
- The game starts even if there are no Mojang assets at all.
