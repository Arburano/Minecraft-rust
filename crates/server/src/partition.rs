//! Who shares a region (T2-06, `server-regions.md`, ADR-0008).
//!
//! Every member (a player, or the world-spawn anchor) owns a square *area*
//! of columns around it: its simulation radius plus the ≥ 1 column buffer.
//! Members whose areas touch or overlap must share a region, so two regions
//! never border each other directly and local interactions (a neighboring
//! block, light, a nearby entity) never leave a region. Members already
//! sharing a region stay together until their areas are [`SPLIT_GAP`]
//! columns apart, so a player walking along a border does not make regions
//! merge and split every tick.
//!
//! Pure functions: the same members in the same order give the same result.

use std::collections::BTreeMap;

use world::ColumnPos;

/// Columns of empty space between two areas at which a shared region splits.
pub const SPLIT_GAP: i32 = 2;

/// Stable region identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(pub u64);

/// One member of the partition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// Stable key (player ID; the empty string is the spawn anchor).
    pub key: String,
    /// The column it stands in.
    pub column: ColumnPos,
    /// Area radius in columns (simulation radius + buffer).
    pub radius: i32,
    /// Its region now, if any.
    pub region: Option<RegionId>,
}

impl Member {
    /// Whether `c` lies in this member's area.
    #[must_use]
    pub fn covers(&self, c: ColumnPos) -> bool {
        (c.x - self.column.x).abs() <= self.radius && (c.z - self.column.z).abs() <= self.radius
    }
}

/// Free columns between the areas of `a` and `b` (negative: they overlap).
#[must_use]
pub fn gap(a: &Member, b: &Member) -> i32 {
    let d = (a.column.x - b.column.x)
        .abs()
        .max((a.column.z - b.column.z).abs());
    d - a.radius - b.radius - 1
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// The new partition: a region ID per member. Existing regions are kept for
/// the group holding most of their members (ties: the lower ID); other
/// groups get fresh IDs from `next_id`.
#[must_use]
pub fn partition(members: &[Member], next_id: &mut u64) -> Vec<RegionId> {
    let n = members.len();
    let mut parent: Vec<usize> = (0..n).collect();
    // Sorting by x lets the pair scan stop early: members farther apart in x
    // than any join distance cannot join.
    let max_r = members.iter().map(|m| m.radius).max().unwrap_or(0);
    let reach = 2 * max_r + 1 + SPLIT_GAP;
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (members[i].column.x, members[i].column.z, i));
    for (k, &i) in order.iter().enumerate() {
        for &j in &order[k + 1..] {
            let (a, b) = (&members[i], &members[j]);
            if b.column.x - a.column.x > reach {
                break;
            }
            let g = gap(a, b);
            let together = a.region.is_some() && a.region == b.region;
            if g < 1 || (together && g < SPLIT_GAP) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    // The smaller index becomes the root: independent of scan order.
                    let (lo, hi) = (ri.min(rj), ri.max(rj));
                    parent[hi] = lo;
                }
            }
        }
    }
    // Groups in member order (roots are their smallest index).
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    // Each group votes for the existing region most of its members are in.
    let mut claims: Vec<(usize, RegionId, usize)> = Vec::new(); // (votes, region, group root) as candidates
    for (&root, idx) in &groups {
        let mut votes: BTreeMap<RegionId, usize> = BTreeMap::new();
        for &i in idx {
            if let Some(r) = members[i].region {
                *votes.entry(r).or_default() += 1;
            }
        }
        for (r, v) in votes {
            claims.push((v, r, root));
        }
    }
    // Strongest claims first; deterministic tie-breaks.
    claims.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut group_region: BTreeMap<usize, RegionId> = BTreeMap::new();
    let mut taken = std::collections::BTreeSet::new();
    for (_, r, root) in claims {
        if !group_region.contains_key(&root) && !taken.contains(&r) {
            group_region.insert(root, r);
            taken.insert(r);
        }
    }
    for &root in groups.keys() {
        group_region.entry(root).or_insert_with(|| {
            let id = RegionId(*next_id);
            *next_id += 1;
            id
        });
    }
    (0..n)
        .map(|i| group_region[&find(&mut parent, i)])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(key: &str, x: i32, region: Option<u64>) -> Member {
        Member {
            key: key.into(),
            column: ColumnPos { x, z: 0 },
            radius: 3,
            region: region.map(RegionId),
        }
    }

    #[test]
    fn touching_areas_share_a_region_and_far_ones_do_not() {
        let mut next = 1;
        // Areas [−3,3] and [4,10] touch (gap 0): one region.
        let r = partition(&[m("a", 0, None), m("b", 7, None)], &mut next);
        assert_eq!(r[0], r[1]);
        // One free column between them (gap 1): separate regions.
        let r = partition(&[m("a", 0, None), m("b", 8, None)], &mut next);
        assert_ne!(r[0], r[1]);
    }

    #[test]
    fn hysteresis_keeps_a_shared_region_until_the_gap_is_wide() {
        let mut next = 10;
        let r = partition(&[m("a", 0, Some(1)), m("b", 8, Some(1))], &mut next);
        assert_eq!(r, vec![RegionId(1), RegionId(1)], "gap 1: stay merged");
        let r = partition(&[m("a", 0, Some(1)), m("b", 9, Some(1))], &mut next);
        assert_ne!(r[0], r[1], "gap 2: split");
        assert!(r.contains(&RegionId(1)), "one half keeps the old ID");
    }

    #[test]
    fn chains_join_and_ids_are_reused_by_majority() {
        let mut next = 100;
        let members = [
            m("a", 0, Some(5)),
            m("b", 7, Some(5)),
            m("c", 14, Some(6)),
            m("d", 40, Some(6)),
        ];
        let r = partition(&members, &mut next);
        assert_eq!(r[0], r[1]);
        assert_eq!(r[1], r[2], "a chain of touching areas is one region");
        assert_eq!(r[0], RegionId(5), "majority keeps its ID");
        assert_eq!(r[3], RegionId(6), "the other group keeps the other ID");
        assert_eq!(next, 100);
    }

    #[test]
    fn order_of_members_does_not_change_the_groups() {
        let a = [
            m("a", 0, None),
            m("b", 7, None),
            m("c", 30, None),
            m("d", 36, None),
        ];
        let mut b = a.clone();
        b.reverse();
        let (mut n1, mut n2) = (1, 1);
        let r1 = partition(&a, &mut n1);
        let r2 = partition(&b, &mut n2);
        let same = |r: &[RegionId], i: usize, j: usize| r[i] == r[j];
        assert!(same(&r1, 0, 1) && same(&r1, 2, 3) && !same(&r1, 0, 2));
        assert!(same(&r2, 3, 2) && same(&r2, 1, 0) && !same(&r2, 3, 1));
    }
}
