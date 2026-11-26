# T2-04. Block edits over the network

- **Status:** todo
- **Depends on:** T2-02, T2-03
- **Owner:** agent (cloud)

## Goal

Dig and place are predicted on the client and validated by the server (reach, occupancy, rate). A rejection rolls the block back. Accepted edits reach every client that has the column; each client relights locally (ADR-0011) and re-meshes through the T1-12 pipeline. Edits persist through the server's storage.

## Acceptance criteria

- [ ] Two clients see each other's edits within one tick plus RTT; a rejected edit reverts on the editing client.
- [ ] Light after network edits equals a fresh lighting on every client (reuse the T1-09/T1-16 equality checks).
- [ ] Edits survive a server restart.

## Out of scope

Inventory and item costs (stage 4).
