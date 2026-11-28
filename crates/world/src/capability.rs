//! Engine capabilities: optional engine features a build switches on through
//! its mods (ADR-0018, `docs/modding/builds.md`).
//!
//! A mod lists the capabilities it needs in `mod.toml`:
//!
//! ```toml
//! [requires]
//! capabilities = ["dimensions", "moving_grids"]
//! ```
//!
//! The build's set is the union over its mods ([`crate::ModSet::capabilities`]).
//! The server and the client start only the subsystems of that set; a
//! capability nobody asked for has no thread, no tick system and no messages.

use std::fmt;

/// One optional engine feature, by its stable name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// More than one world per server, travel between them.
    Dimensions,
    /// Moving block structures with rigid-body physics (ADR-0019).
    MovingGrids,
    /// Large-distance coordinates, vacuum, orbit-scale travel.
    Space,
    /// Named world areas with rules.
    Zones,
    /// Private copies of an area per party.
    Instances,
    /// Server-stored per-player stats, levels and skills.
    Progression,
}

impl Capability {
    /// Every capability, in a fixed order (also the bit order of
    /// [`Capabilities`]).
    pub const ALL: [Self; 6] = [
        Self::Dimensions,
        Self::MovingGrids,
        Self::Space,
        Self::Zones,
        Self::Instances,
        Self::Progression,
    ];

    /// The stable name used in `mod.toml`, `server.toml` and the protocol.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Dimensions => "dimensions",
            Self::MovingGrids => "moving_grids",
            Self::Space => "space",
            Self::Zones => "zones",
            Self::Instances => "instances",
            Self::Progression => "progression",
        }
    }

    /// The capability with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The cargo feature a binary needs to ship this capability, for the
    /// capabilities with a heavy dependency (ADR-0018 "not shipped"). The
    /// feature has the capability's name.
    #[must_use]
    pub const fn cargo_feature(self) -> Option<&'static str> {
        match self {
            Self::MovingGrids => Some("moving_grids"),
            _ => None,
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of capabilities. Small and `Copy`: checking it on a hot path costs
/// one bit test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Capabilities(u8);

impl Capabilities {
    /// No capability: the classic game.
    pub const NONE: Self = Self(0);

    /// Every capability.
    #[must_use]
    pub const fn all() -> Self {
        let mut bits = 0;
        let mut i = 0;
        while i < Capability::ALL.len() {
            bits |= Capability::ALL[i].bit();
            i += 1;
        }
        Self(bits)
    }

    /// The set with one more capability.
    #[must_use]
    pub const fn with(self, c: Capability) -> Self {
        Self(self.0 | c.bit())
    }

    /// The set without a capability.
    #[must_use]
    pub const fn without(self, c: Capability) -> Self {
        Self(self.0 & !c.bit())
    }

    /// Whether the set has a capability.
    #[must_use]
    pub const fn contains(self, c: Capability) -> bool {
        self.0 & c.bit() != 0
    }

    /// Whether the set is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Both sets together.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The capabilities of `self` that `other` lacks.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// The capabilities, in [`Capability::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = Capability> {
        Capability::ALL
            .into_iter()
            .filter(move |c| self.contains(*c))
    }

    /// The names, in [`Capability::ALL`] order.
    #[must_use]
    pub fn names(self) -> Vec<String> {
        self.iter().map(|c| c.name().to_owned()).collect()
    }

    /// Parses names; the error is the first unknown name.
    pub fn from_names<S: AsRef<str>>(names: &[S]) -> Result<Self, String> {
        names.iter().try_fold(Self::NONE, |set, n| {
            Capability::from_name(n.as_ref())
                .map(|c| set.with(c))
                .ok_or_else(|| n.as_ref().to_owned())
        })
    }
}

impl FromIterator<Capability> for Capabilities {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        iter.into_iter().fold(Self::NONE, Self::with)
    }
}

impl fmt::Display for Capabilities {
    /// `dimensions, moving_grids`, or `none`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("none");
        }
        for (i, c) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(c.name())?;
        }
        Ok(())
    }
}

/// The known names, for error messages: `"dimensions", "moving_grids", …`.
#[must_use]
pub fn known_names() -> String {
    Capability::ALL
        .iter()
        .map(|c| format!("{:?}", c.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for c in Capability::ALL {
            assert_eq!(Capability::from_name(c.name()), Some(c));
        }
        assert_eq!(Capability::from_name("weather"), None);
        assert_eq!(Capability::from_name("Dimensions"), None);
    }

    #[test]
    fn set_operations() {
        let a = Capabilities::NONE
            .with(Capability::Dimensions)
            .with(Capability::Zones);
        let b: Capabilities = [Capability::Zones, Capability::MovingGrids]
            .into_iter()
            .collect();
        assert!(a.contains(Capability::Zones) && !a.contains(Capability::Space));
        assert_eq!(
            a.union(b).iter().collect::<Vec<_>>(),
            [
                Capability::Dimensions,
                Capability::MovingGrids,
                Capability::Zones
            ]
        );
        assert_eq!(
            a.difference(b),
            Capabilities::NONE.with(Capability::Dimensions)
        );
        assert_eq!(
            a.without(Capability::Zones).without(Capability::Dimensions),
            Capabilities::NONE
        );
        assert_eq!(Capabilities::all().iter().count(), Capability::ALL.len());
        assert_eq!(a.to_string(), "dimensions, zones");
        assert_eq!(Capabilities::NONE.to_string(), "none");
        assert_eq!(Capabilities::from_names(&a.names()), Ok(a));
        assert_eq!(
            Capabilities::from_names(&["zones", "lasers"]),
            Err("lasers".to_owned())
        );
    }
}
