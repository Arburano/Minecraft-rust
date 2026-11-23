# T2-03. Player movement: prediction, reconciliation, entity replication

- **Status:** todo
- **Depends on:** T2-02, T1-12 (deterministic physics)
- **Owner:** agent (cloud) + owner's feel review

## Goal

[simulation-and-ai.md](../../docs/architecture/simulation-and-ai.md), "Prediction and reconciliation":

- clients send intents with tick numbers over datagrams (sent 2–3 times against loss);
- the server runs the same `sim::physics` as the authority and acknowledges ticks;
- clients predict, and on divergence roll back and replay with a smoothing threshold;
- other players and entities arrive as deltas against the last acknowledged snapshot, with interest management (20 Hz near, 5–10 Hz far).

## Acceptance criteria

- [ ] **Determinism:** client and server physics agree bit for bit on the same inputs (test across both builds).
- [ ] **Loss:** at 2% loss / RTT 100 ms the local player shows no visible correction in a scripted walk; a forced divergence corrects smoothly.
- [ ] **Traffic:** ≤ 50 kbit/s per player with 20 moving entities nearby in steady state (measured).
- [ ] **Feel:** the owner plays two clients on LAN and reviews the feel.

## Out of scope

Mobs and AI (stage 4); anti-cheat (stage 7).
