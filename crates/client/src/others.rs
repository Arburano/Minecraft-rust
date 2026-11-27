//! Other players (and later all entities) seen on a server (T2-03).
//!
//! Server datagrams are deltas against acknowledged snapshots; the
//! [`Reconstructor`] turns them back into full snapshots. Each entity's drawn
//! position glides from where it was drawn to its newest position over the
//! time between its last two updates, so near entities (20 Hz) and far ones
//! (5 Hz) both move smoothly without a fixed render delay.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use glam::DVec3;
use protocol::{EntityDatagram, EntityUpdate, NetId};
use server::replication::{EntityState, Reconstructor, dequantize};

/// Shortest and longest glide between two updates.
const MIN_GLIDE: Duration = Duration::from_millis(50);
const MAX_GLIDE: Duration = Duration::from_millis(300);
/// A jump farther than this (blocks) is a teleport: no glide.
const TELEPORT: f64 = 8.0;

/// Another entity, as drawn this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OtherEntity {
    /// Network identity.
    pub id: NetId,
    /// Entity type (0 = player).
    pub kind: u16,
    /// Feet position, interpolated.
    pub pos: DVec3,
    /// Body yaw, radians.
    pub yaw: f32,
    /// Flying.
    pub flying: bool,
}

struct Track {
    kind: u16,
    from: DVec3,
    to: DVec3,
    yaw_from: f32,
    yaw_to: f32,
    start: Instant,
    glide: Duration,
    last_change: Instant,
    flags: u8,
}

impl Track {
    fn at(&self, now: Instant) -> (DVec3, f32) {
        let t = if self.glide.is_zero() {
            1.0
        } else {
            (now.saturating_duration_since(self.start).as_secs_f64() / self.glide.as_secs_f64())
                .min(1.0)
        };
        // Shortest way around for the yaw.
        let mut dy = self.yaw_to - self.yaw_from;
        dy -= std::f32::consts::TAU * (dy / std::f32::consts::TAU).round();
        (self.from.lerp(self.to, t), self.yaw_from + dy * t as f32)
    }
}

/// The entities the server reports to this client.
#[derive(Default)]
pub struct Others {
    recon: Reconstructor,
    tracks: HashMap<NetId, Track>,
}

impl Others {
    /// Applies a server datagram.
    pub fn apply(&mut self, d: &EntityDatagram, now: Instant) {
        let Some((view, changed)) = self.recon.apply(d) else {
            return;
        };
        for id in changed {
            match view.get(&id) {
                None => {
                    self.tracks.remove(&id);
                }
                Some(s) => {
                    let (to, yaw_to) = position(s);
                    match self.tracks.get_mut(&id) {
                        Some(t) => {
                            let (from, yaw_from) = t.at(now);
                            let since = now.saturating_duration_since(t.last_change);
                            let far = from.distance(to) > TELEPORT;
                            t.from = if far { to } else { from };
                            t.yaw_from = yaw_from;
                            t.to = to;
                            t.yaw_to = yaw_to;
                            t.start = now;
                            t.glide = if far {
                                Duration::ZERO
                            } else {
                                since.clamp(MIN_GLIDE, MAX_GLIDE)
                            };
                            t.last_change = now;
                            t.kind = s.kind;
                            t.flags = s.flags;
                        }
                        None => {
                            self.tracks.insert(
                                id,
                                Track {
                                    kind: s.kind,
                                    from: to,
                                    to,
                                    yaw_from: yaw_to,
                                    yaw_to,
                                    start: now,
                                    glide: Duration::ZERO,
                                    last_change: now,
                                    flags: s.flags,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    /// The newest snapshot to acknowledge.
    #[must_use]
    pub fn latest(&self) -> u32 {
        self.recon.latest()
    }

    /// Entities as drawn at `now`.
    pub fn at(&self, now: Instant) -> impl Iterator<Item = OtherEntity> + '_ {
        self.tracks.iter().map(move |(id, t)| {
            let (pos, yaw) = t.at(now);
            OtherEntity {
                id: *id,
                kind: t.kind,
                pos,
                yaw,
                flying: t.flags & EntityUpdate::FLYING != 0,
            }
        })
    }

    /// Number of entities known.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Whether no entity is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

fn position(s: &EntityState) -> (DVec3, f32) {
    (
        DVec3::from_array(dequantize(s.pos)),
        s.look.yaw.to_radians(),
    )
}
