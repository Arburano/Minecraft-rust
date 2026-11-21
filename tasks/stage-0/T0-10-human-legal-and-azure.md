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

Until the answers arrive, development assumes "yes" to open questions 4 and 5 (see [open-questions.md](../../docs/open-questions.md)). The development rules in [docs/legal.md](../../docs/legal.md) stay in force unchanged.

## Out of scope

The agent does not perform this task; it may only prepare a draft letter to the lawyer.
