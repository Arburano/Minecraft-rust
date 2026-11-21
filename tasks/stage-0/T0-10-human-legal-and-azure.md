# T0-10. Lawyer and Minecraft API access

- **Status:** in-progress (2026-11-21: the owner has sent the requests — waiting for answers)
- **Depends on:** —
- **Owner:** human (not an agent)

## Goal

Register an Azure application and submit a form for Minecraft API access; obtain legal advice on the list of questions.

## Specs

- [docs/legal.md](../../docs/legal.md)
- [docs/architecture/auth-and-assets.md](../../docs/architecture/auth-and-assets.md)
- [docs/open-questions.md](../../docs/open-questions.md)

## Deliverables

- Approved client id (stored in secrets, not in the repository).
- Lawyer's opinion on the questions from docs/legal.md.

## Acceptance criteria

- [ ] Open questions No. 4 and No. 5 are closed or reformulated.

## Interim owner decision (2026-11-21)

Development does not wait for the answers: offline mode replaces Microsoft login for testing, and developers test with assets from their own licensed installation (see [open-questions.md](../../docs/open-questions.md) No. 4 and 5 and [docs/legal.md](../../docs/legal.md) rules 2 and 7). Nothing is launched publicly until the answers arrive.

## Out of scope

The agent does not perform this task; it may only prepare a draft letter to the lawyer.
