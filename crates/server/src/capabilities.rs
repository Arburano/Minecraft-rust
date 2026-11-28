//! Optional engine subsystems, started only for the capabilities the build's
//! mods require (ADR-0018, `docs/modding/builds.md`).
//!
//! The server asks the build for its capability set
//! ([`world::ModSet::capabilities`]) and starts one [`Subsystem`] per
//! capability from a [`SubsystemRegistry`]. A capability nobody requires has
//! nothing started: no thread, no per-tick call, no protocol messages. With no
//! capability the world thread's per-tick cost is one check of an empty list.
//!
//! The registry maps a capability to a factory. Capability crates (the first
//! is `moving_grids`, ADR-0019) register theirs in
//! [`SubsystemRegistry::builtin`]; tests register their own.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use world::{Capabilities, Capability, ModSet};

/// The capabilities this server binary was compiled with. A capability with a
/// heavy dependency sits behind the cargo feature of the same name
/// ([`Capability::cargo_feature`]); the others are always compiled.
#[must_use]
pub const fn compiled() -> Capabilities {
    let all = Capabilities::all();
    if cfg!(feature = "moving_grids") {
        all
    } else {
        all.without(Capability::MovingGrids)
    }
}

/// A running optional subsystem. It lives on the world thread.
pub trait Subsystem: Send {
    /// Runs once per world tick, after the regions simulate.
    fn tick(&mut self, tick: u64);
    /// Called once when the world stops (save, join threads).
    fn shutdown(&mut self) {}
    /// The zone index regions read, for the `zones` subsystem (T4-08).
    fn zones(&self) -> Option<Arc<world::zones::ZoneIndex>> {
        None
    }
    /// The handle that changes the world's zones, for the `zones` subsystem.
    fn zone_control(&self) -> Option<crate::zones::ZoneControl> {
        None
    }
}

/// What a factory gets to start its subsystem.
pub struct SubsystemContext<'a> {
    /// The build.
    pub mods: &'a ModSet,
    /// The world folder: a subsystem keeps its saved data under
    /// `<world>/<capability>/`.
    pub world_dir: &'a Path,
}

/// Starts the subsystem of one capability.
pub type SubsystemFactory =
    Arc<dyn Fn(&SubsystemContext<'_>) -> Result<Box<dyn Subsystem>, String> + Send + Sync>;

/// Which subsystem starts for which capability.
#[derive(Clone, Default)]
pub struct SubsystemRegistry {
    factories: BTreeMap<Capability, SubsystemFactory>,
}

impl SubsystemRegistry {
    /// The subsystems this engine version implements: `zones` (T4-08).
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self::default();
        crate::zones::Zones::register(&mut r);
        r
    }

    /// Registers (or replaces) the factory of a capability.
    pub fn register(&mut self, capability: Capability, factory: SubsystemFactory) {
        self.factories.insert(capability, factory);
    }

    /// The capabilities with a factory.
    #[must_use]
    pub fn implemented(&self) -> Capabilities {
        self.factories.keys().copied().collect()
    }

    /// Starts the subsystems of the build's capabilities, in
    /// [`Capability::ALL`] order. Fails on a capability that is not compiled
    /// in or has no factory; the error names the build, the mod and the
    /// capability.
    pub fn start(&self, ctx: &SubsystemContext<'_>) -> Result<Subsystems, String> {
        ctx.mods
            .check_compiled(compiled(), "server")
            .map_err(|e| e.to_string())?;
        let wanted = ctx.mods.capabilities();
        let mut active = Vec::new();
        for capability in wanted.iter() {
            let Some(factory) = self.factories.get(&capability) else {
                let by = ctx.mods.requirer(capability).map_or("?", |m| m.id.as_str());
                return Err(format!(
                    "build {} needs capability {capability} (required by mod {by}), \
                     which this engine version does not implement yet",
                    ctx.mods.build()
                ));
            };
            let subsystem = factory(ctx).map_err(|e| {
                format!(
                    "build {}: capability {capability} did not start: {e}",
                    ctx.mods.build()
                )
            })?;
            active.push((capability, subsystem));
        }
        Ok(Subsystems {
            set: wanted,
            active,
        })
    }
}

impl fmt::Debug for SubsystemRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SubsystemRegistry({})", self.implemented())
    }
}

impl PartialEq for SubsystemRegistry {
    /// Equal when the same capabilities have factories (factories are not
    /// comparable).
    fn eq(&self, other: &Self) -> bool {
        self.implemented() == other.implemented()
    }
}

impl Eq for SubsystemRegistry {}

/// The started subsystems of a world.
#[derive(Default)]
pub struct Subsystems {
    set: Capabilities,
    active: Vec<(Capability, Box<dyn Subsystem>)>,
}

impl Subsystems {
    /// The capabilities that run.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.set
    }

    /// How many subsystems run.
    #[must_use]
    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// Whether none runs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// The current zone index, when `zones` runs.
    #[must_use]
    pub fn zones(&self) -> Option<Arc<world::zones::ZoneIndex>> {
        self.active.iter().find_map(|(_, s)| s.zones())
    }

    /// The zone API handle, when `zones` runs.
    #[must_use]
    pub fn zone_control(&self) -> Option<crate::zones::ZoneControl> {
        self.active.iter().find_map(|(_, s)| s.zone_control())
    }

    /// One world tick for every subsystem.
    pub fn tick(&mut self, tick: u64) {
        for (_, s) in &mut self.active {
            s.tick(tick);
        }
    }

    /// Stops every subsystem, in reverse start order. Runs once: later calls
    /// (and the drop) do nothing.
    pub fn shutdown(&mut self) {
        while let Some((_, mut s)) = self.active.pop() {
            s.shutdown();
        }
    }
}

impl Drop for Subsystems {
    /// A world that fails to start still stops what already started.
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl fmt::Debug for Subsystems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Subsystems({})", self.set)
    }
}

/// The capabilities a world's saved data needs, as stored in `server.toml`,
/// checked against the build. Returns the new list to store: the union, since
/// data written once stays in the world (ADR-0018, rule 4).
pub fn check_world(
    stored: &[String],
    build: &ModSet,
    world: &Path,
) -> Result<Capabilities, String> {
    let saved = Capabilities::from_names(stored).map_err(|bad| {
        format!(
            "{}: this world needs capability {bad:?}, unknown to this engine version; \
             update the game",
            world.join("server.toml").display()
        )
    })?;
    let have = build.capabilities();
    if let Some(missing) = saved.difference(have).iter().next() {
        return Err(format!(
            "this world has saved data of capability {missing}, but build {} does not have it \
             (its mods require: {have}); open the world with a build whose mods require {missing}",
            build.build()
        ));
    }
    Ok(saved.union(have))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_starts_nothing() {
        let mods = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
        let dir = std::env::temp_dir();
        let subs = SubsystemRegistry::builtin()
            .start(&SubsystemContext {
                mods: &mods,
                world_dir: &dir,
            })
            .expect("start");
        assert!(subs.is_empty() && subs.capabilities().is_empty());
        assert_eq!(check_world(&[], &mods, &dir), Ok(Capabilities::NONE));
        let err = check_world(&["zones".into()], &mods, &dir).expect_err("zones");
        assert!(
            err.contains("capability zones") && err.contains("build classic"),
            "{err}"
        );
        let err = check_world(&["lasers".into()], &mods, &dir).expect_err("unknown");
        assert!(err.contains("\"lasers\""), "{err}");
    }

    #[test]
    fn compiled_set_follows_the_features() {
        assert_eq!(
            compiled().contains(Capability::MovingGrids),
            cfg!(feature = "moving_grids")
        );
        assert!(compiled().contains(Capability::Zones));
    }
}
