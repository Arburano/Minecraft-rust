//! Which columns a client gets, and in which order (T2-02 streaming rules).
//!
//! - The server streams every column within Chebyshev distance `view` of
//!   the player's column ([`ClientView`]).
//! - When the player moves, columns farther than `view + FORGET_MARGIN` are
//!   forgotten: if the player comes back they are sent again. The client
//!   keeps columns one column longer than that (its loader unloads beyond
//!   `view + 2`), so its own predicted position may differ from the server's
//!   by a column without leaving holes.
//! - The connection's [`SendQueue`] sends the nearest pending column first
//!   and drops pending columns the player has moved away from.

use std::collections::HashSet;

use bytes::Bytes;
use world::ColumnPos;

/// Columns beyond `view + FORGET_MARGIN` are forgotten (resent on return).
pub const FORGET_MARGIN: i32 = 1;

/// Chebyshev (square) distance between columns.
#[must_use]
pub fn chebyshev(a: ColumnPos, b: ColumnPos) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

/// Squared Euclidean distance between columns (send order inside the square).
#[must_use]
pub fn dist2(a: ColumnPos, b: ColumnPos) -> i64 {
    let (dx, dz) = (i64::from(a.x - b.x), i64::from(a.z - b.z));
    dx * dx + dz * dz
}

/// What one client has been sent (world thread side).
#[derive(Debug)]
pub struct ClientView {
    center: ColumnPos,
    view: i32,
    delivered: HashSet<ColumnPos>,
}

impl ClientView {
    /// A client at `center` streaming `view` columns around.
    #[must_use]
    pub fn new(center: ColumnPos, view: i32) -> Self {
        Self {
            center,
            view,
            delivered: HashSet::new(),
        }
    }

    /// The player's column.
    #[must_use]
    pub fn center(&self) -> ColumnPos {
        self.center
    }

    /// Stream radius.
    #[must_use]
    pub fn view(&self) -> i32 {
        self.view
    }

    /// Moves the center and forgets columns beyond `view + FORGET_MARGIN`.
    /// Returns whether the center changed.
    pub fn set_center(&mut self, center: ColumnPos) -> bool {
        if center == self.center {
            return false;
        }
        self.center = center;
        let keep = self.view + FORGET_MARGIN;
        self.delivered.retain(|&c| chebyshev(c, center) <= keep);
        true
    }

    /// Whether `pos` is in range and not yet sent.
    #[must_use]
    pub fn wants(&self, pos: ColumnPos) -> bool {
        chebyshev(pos, self.center) <= self.view && !self.delivered.contains(&pos)
    }

    /// Records that `pos` was handed to the connection.
    pub fn mark(&mut self, pos: ColumnPos) {
        self.delivered.insert(pos);
    }

    /// Every column in range, nearest first.
    #[must_use]
    pub fn in_range(&self) -> Vec<ColumnPos> {
        let (c, v) = (self.center, self.view);
        let mut out: Vec<ColumnPos> = (c.x - v..=c.x + v)
            .flat_map(|x| (c.z - v..=c.z + v).map(move |z| ColumnPos { x, z }))
            .collect();
        out.sort_by_key(|&p| dist2(p, c));
        out
    }

    /// Columns sent and still remembered.
    #[must_use]
    pub fn delivered_len(&self) -> usize {
        self.delivered.len()
    }
}

/// Columns waiting to be written to a client's streams (connection side).
#[derive(Debug)]
pub struct SendQueue {
    center: ColumnPos,
    keep: i32,
    items: Vec<(ColumnPos, Bytes)>,
}

impl SendQueue {
    /// A queue for a client streaming `view` columns around.
    #[must_use]
    pub fn new(view: i32) -> Self {
        Self {
            center: ColumnPos { x: 0, z: 0 },
            keep: view + FORGET_MARGIN,
            items: Vec::new(),
        }
    }

    /// Moves the center; drops columns the world thread has forgotten too
    /// (same rule as [`ClientView::set_center`]).
    pub fn set_center(&mut self, center: ColumnPos) {
        self.center = center;
        let keep = self.keep;
        self.items.retain(|(p, _)| chebyshev(*p, center) <= keep);
    }

    /// Adds a framed column.
    pub fn push(&mut self, pos: ColumnPos, frame: Bytes) {
        self.items.push((pos, frame));
    }

    /// Pending columns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether nothing is pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Takes the nearest pending column with its stream priority (higher is
    /// sent first; nearer columns get higher values).
    pub fn pop_nearest(&mut self) -> Option<(ColumnPos, Bytes, i32)> {
        let center = self.center;
        let (i, _) = self
            .items
            .iter()
            .enumerate()
            .min_by_key(|(_, (p, _))| dist2(*p, center))?;
        let (pos, frame) = self.items.swap_remove(i);
        let priority = -i32::try_from(dist2(pos, center)).unwrap_or(i32::MAX);
        Some((pos, frame, priority))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(x: i32, z: i32) -> ColumnPos {
        ColumnPos { x, z }
    }

    #[test]
    fn view_sends_once_and_resends_after_leaving() {
        let mut v = ClientView::new(c(0, 0), 3);
        assert_eq!(v.in_range().len(), 49);
        assert_eq!(v.in_range()[0], c(0, 0), "nearest first");
        assert!(v.wants(c(3, -3)));
        assert!(!v.wants(c(4, 0)), "outside the view");
        v.mark(c(3, 0));
        v.mark(c(-3, 0));
        assert!(!v.wants(c(3, 0)), "sent once");
        // Moving by one keeps everything within view + margin.
        assert!(v.set_center(c(1, 0)));
        assert!(!v.set_center(c(1, 0)), "no change");
        assert_eq!(v.delivered_len(), 2);
        // Moving further forgets (-3, 0): distance 5 > 3 + 1.
        v.set_center(c(2, 0));
        assert_eq!(v.delivered_len(), 1);
        // Coming back: (-3, 0) is wanted again, (3, 0) is not.
        v.set_center(c(0, 0));
        assert!(v.wants(c(-3, 0)));
        assert!(!v.wants(c(3, 0)));
    }

    #[test]
    fn queue_sends_nearest_and_drops_forgotten() {
        let mut q = SendQueue::new(2);
        for (x, z) in [(2, 2), (0, 1), (-1, 0), (0, 0), (2, 0)] {
            q.push(c(x, z), Bytes::from_static(b"col"));
        }
        let (p, _, prio) = q.pop_nearest().expect("pending");
        assert_eq!((p, prio), (c(0, 0), 0));
        let (p, _, prio) = q.pop_nearest().expect("pending");
        assert!(p == c(0, 1) || p == c(-1, 0));
        assert_eq!(prio, -1);
        // Moving to (5, 0): keep = 3; only (2, 0) and (2, 2) stay.
        q.set_center(c(5, 0));
        assert_eq!(q.len(), 2);
        let (p, _, _) = q.pop_nearest().expect("pending");
        assert_eq!(p, c(2, 0));
        let (p, _, _) = q.pop_nearest().expect("pending");
        assert_eq!(p, c(2, 2));
        assert!(q.pop_nearest().is_none());
        assert!(q.is_empty());
    }

    #[test]
    fn queue_and_view_forget_alike() {
        // Whatever the view forgets, the queue drops, so a forgotten column
        // is never left neither sent nor resent.
        let mut v = ClientView::new(c(0, 0), 4);
        let mut q = SendQueue::new(4);
        q.set_center(c(0, 0));
        for p in v.in_range() {
            v.mark(p);
            q.push(p, Bytes::new());
        }
        for step in [c(1, 0), c(3, 1), c(7, -2), c(2, 2)] {
            v.set_center(step);
            q.set_center(step);
            let mut pending: Vec<ColumnPos> = Vec::new();
            let mut probe = SendQueue::new(4);
            std::mem::swap(&mut probe, &mut q);
            while let Some((p, f, _)) = probe.pop_nearest() {
                pending.push(p);
                q.push(p, f);
            }
            q.set_center(step);
            for p in pending {
                assert!(
                    !v.wants(p),
                    "{p:?} pending in the queue must still count as sent"
                );
            }
        }
    }
}
