# T1-08. Offline mode for client and server

- **Status:** done (2026-11-21)
- **Depends on:** T0-10 owner decision (legal.md rule 7)
- **Owner:** agent (cloud)

## Goal

Development and tests do not wait for Microsoft API approval: implement offline mode exactly as specified (local player name, deterministic offline UUID, no Microsoft/Mojang network calls), off by default, server only with an explicit flag, LAN/localhost bind by default.

## Specs

- [auth-and-assets.md](../../docs/architecture/auth-and-assets.md) — Offline mode
- [legal.md](../../docs/legal.md) — rule 7
- [open questions](../../docs/open-questions.md) No. 4

## Deliverables

- `auth::Identity` with `Offline { name, uuid }` and a placeholder for `Microsoft` (implemented when T0-10 is answered).
- Client setting/CLI flag for offline mode; server `--offline` flag with warning log and localhost/LAN default bind.
- Tests: offline UUID determinism; server refuses offline clients without the flag; no outbound network calls in offline mode (test with a blocked network or a mock).

## Acceptance criteria

- [x] Tests pass; offline mode cannot be enabled on a server without the explicit flag.
- [x] Docs updated if anything differs from the spec.

## Out of scope

Microsoft login chain (after T0-10), session JWT service (stage 3).
