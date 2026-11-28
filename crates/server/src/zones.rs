//! The `zones` capability (T4-08, PRD §14 No. 11, `docs/modding/zones.md`).
//!
//! Zones come from the build's mods (`data/zones/*.toml`) and from the world
//! (`<world>/zones/zones.toml`); a world zone with the id of a mod zone
//! replaces it. The subsystem keeps a [`ZoneIndex`] that regions read during
//! `Simulate` (shared, immutable); [`ZoneControl`] changes the world's zones,
//! and the change takes effect between two ticks.
//!
//! What the server enforces with it:
//! - **build:** a denied edit is refused (`BlockAck { accepted: false }`, the
//!   client rolls its prediction back);
//! - **entry:** a step that would take a player from an allowed cell into a
//!   denied one is undone, and the client is corrected;
//! - **PvP and spawning:** [`ZoneIndex::pvp_allowed`] and
//!   [`ZoneIndex::spawn_table`], for the combat and spawn systems of the
//!   gameplay stage (neither exists yet, see the T4-08 report).
//!
//! Entities other than players cross zone borders freely (only spawning is
//! restricted by zones, PRD §14 No. 11).

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use world::Capability;
use world::zones::{self, ZoneDef, ZoneError, ZoneIndex};

use crate::capabilities::{Subsystem, SubsystemContext, SubsystemFactory, SubsystemRegistry};

/// A change to the world's zones.
#[derive(Debug)]
enum Command {
    Define(ZoneDef),
    Remove(String),
}

/// Changes the world's own zones (the server-side zone API; mods reach it
/// through the mod API once it exists, stage 3). Cheap to clone; changes
/// apply at the end of the next world tick and are saved to
/// `<world>/zones/zones.toml`.
#[derive(Clone, Debug)]
pub struct ZoneControl {
    tx: Sender<Command>,
}

impl ZoneControl {
    /// Adds a zone to the world, or replaces the world zone or mod zone with
    /// the same id. The definition is checked here.
    pub fn define(&self, zone: ZoneDef) -> Result<(), ZoneError> {
        zone.validate()?;
        self.tx
            .send(Command::Define(zone))
            .map_err(|_| ZoneError("the world has stopped".into()))
    }

    /// Removes a zone the world defined; a mod zone it replaced comes back.
    /// Unknown ids are ignored.
    pub fn remove(&self, id: &str) -> Result<(), ZoneError> {
        self.tx
            .send(Command::Remove(id.to_owned()))
            .map_err(|_| ZoneError("the world has stopped".into()))
    }
}

/// The running `zones` subsystem.
pub struct Zones {
    world_dir: PathBuf,
    from_mods: Vec<ZoneDef>,
    own: Vec<ZoneDef>,
    index: Arc<ZoneIndex>,
    rx: Receiver<Command>,
    control: ZoneControl,
}

impl Zones {
    /// Loads the build's and the world's zones.
    pub fn start(ctx: &SubsystemContext<'_>) -> Result<Self, String> {
        let from_mods = zones::from_mods(ctx.mods).map_err(|e| e.0)?;
        let own = zones::read_world(ctx.world_dir).map_err(|e| e.0)?;
        let index = Arc::new(merge(&from_mods, &own).map_err(|e| e.0)?);
        let (tx, rx) = crossbeam_channel::unbounded();
        Ok(Self {
            world_dir: ctx.world_dir.to_path_buf(),
            from_mods,
            own,
            index,
            rx,
            control: ZoneControl { tx },
        })
    }

    /// The factory registered for [`Capability::Zones`].
    #[must_use]
    pub fn factory() -> SubsystemFactory {
        Arc::new(|ctx| Ok(Box::new(Self::start(ctx)?) as Box<dyn Subsystem>))
    }

    /// Registers the factory.
    pub fn register(registry: &mut SubsystemRegistry) {
        registry.register(Capability::Zones, Self::factory());
    }
}

/// Mod zones with the world's on top (same id: the world's wins).
fn merge(from_mods: &[ZoneDef], own: &[ZoneDef]) -> Result<ZoneIndex, ZoneError> {
    let mut all: Vec<ZoneDef> = from_mods
        .iter()
        .filter(|m| !own.iter().any(|o| o.id == m.id))
        .cloned()
        .collect();
    all.extend(own.iter().cloned());
    ZoneIndex::build(all)
}

impl Subsystem for Zones {
    fn tick(&mut self, _tick: u64) {
        let mut changed = false;
        while let Ok(cmd) = self.rx.try_recv() {
            let mut next = self.own.clone();
            match cmd {
                Command::Define(z) => match next.iter_mut().find(|o| o.id == z.id) {
                    Some(slot) => *slot = z,
                    None => next.push(z),
                },
                Command::Remove(id) => next.retain(|o| o.id != id),
            }
            match merge(&self.from_mods, &next) {
                Ok(index) => {
                    self.own = next;
                    self.index = Arc::new(index);
                    changed = true;
                }
                Err(e) => eprintln!("server: zone change refused: {e}"),
            }
        }
        // Zone changes are rare operator or mod actions; the file is small.
        if changed && let Err(e) = zones::write_world(&self.world_dir, &self.own) {
            eprintln!("server: saving zones: {e}");
        }
    }

    fn zones(&self) -> Option<Arc<ZoneIndex>> {
        Some(Arc::clone(&self.index))
    }

    fn zone_control(&self) -> Option<ZoneControl> {
        Some(self.control.clone())
    }
}
