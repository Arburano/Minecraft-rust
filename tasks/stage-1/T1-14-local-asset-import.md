# T1-14. Import assets from a local licensed installation

- **Status:** todo
- **Depends on:** T1-07, open question 5 (owner decision)
- **Owner:** agent (cloud for code, owner's laptop for real-asset tests)

## Goal

Let developers and players see real textures from **their own** licensed installation: read the client jar and asset index from a local path, convert into the T1-07 cache. Never commit, upload to CI or bundle these files.

## Specs

- [auth-and-assets.md](../../docs/architecture/auth-and-assets.md) — Local installation for development
- [legal.md](../../docs/legal.md) — rule 2
- [open questions](../../docs/open-questions.md) No. 5

## Deliverables

- Reader for a local installation path (setting or `ASSETS_LOCAL_INSTALL`), version selection, conversion into the cache.
- Opt-in tests that skip with a message when the variable is not set; CI stays on fallback assets.
- Download from the official manifest after an ownership check — only after T0-10 is answered (stub + TODO referencing open question 5).

## Acceptance criteria

- [ ] Opt-in tests pass on the owner's laptop; CI skips them.
- [ ] A `.gitignore`/CI guard prevents committing converted or original Mojang files (test or check script).
- [ ] Conversion time measured.

## Out of scope

Downloading assets for players before T0-10 is answered; sounds/models beyond what the renderer uses.
