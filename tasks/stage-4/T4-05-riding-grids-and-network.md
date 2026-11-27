# T4-05. Riding grids and grid replication

- **Status:** todo
- **Depends on:** T4-03, T2-03, T2-07
- **Owner:** agent (cloud) + owner feel review

## Goal

Players and mobs walk on moving grids; clients see grids move smoothly.

## Deliverables

- Attaching and detaching entities; movement in the grid frame with a frame force; saving players relative to a grid.
- Protocol: grid records, pose/velocity at 20 Hz in the entity channel, edits with a grid id, created/removed/split events; `EntityUpdate` frame field.
- Prediction and reconciliation in the grid frame.

## Acceptance criteria

- [ ] On a grid turning at 1 rad/s and moving at 30 m/s a player stays on deck, walks and jumps normally (test + owner review).
- [ ] Bandwidth per moving grid measured and recorded in performance-budgets.
