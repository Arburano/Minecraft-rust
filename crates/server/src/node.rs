//! Several dimensions on one server (T4-02, `docs/architecture/dimensions.md`).
//!
//! The node owns one [`World`] per dimension. Each has its own regions,
//! column store, generator, cache and physics; regions never span
//! dimensions. The node's thread routes every message from the connections
//! to the world the player is in, then ticks each world in turn. A player
//! travels between dimensions between ticks: it leaves one world (its
//! queued intents applied, as when leaving the server) and enters the other,
//! whose `Spawn` tells the client to start its world again. Player files
//! record the dimension, so a returning player joins where it left.
//!
//! A build with one dimension runs the world directly, without the node.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use crate::game::{TICK_RATE, ToConn, ToWorld, World};
use crate::players;

/// The command offline servers accept in chat to travel (development and
/// tests; gameplay travel comes from mods, e.g. portals).
pub const TRAVEL_COMMAND: &str = "/dimension";

/// Worlds by dimension, and where each player is.
pub struct Node {
    worlds: Vec<World>,
    /// Player id → index into `worlds`.
    at: HashMap<String, usize>,
    /// Where player files live (the world folder).
    dir: PathBuf,
    /// Whether players may travel with [`TRAVEL_COMMAND`].
    commands: bool,
    travels: u64,
}

impl Node {
    /// A node over `worlds` (the overworld first: new players start there).
    #[must_use]
    pub fn new(worlds: Vec<World>, dir: PathBuf, commands: bool) -> Self {
        Self {
            worlds,
            at: HashMap::new(),
            dir,
            commands,
            travels: 0,
        }
    }

    /// The dimensions, in the order of their worlds.
    #[must_use]
    pub fn dimensions(&self) -> Vec<String> {
        self.worlds
            .iter()
            .map(|w| w.dimension().to_owned())
            .collect()
    }

    /// The dimension a player is in.
    #[must_use]
    pub fn player_dimension(&self, id: &str) -> Option<&str> {
        self.at.get(id).map(|&i| self.worlds[i].dimension())
    }

    /// Travels done so far.
    #[must_use]
    pub fn travels(&self) -> u64 {
        self.travels
    }

    /// The worlds (tests and statistics).
    #[must_use]
    pub fn worlds(&self) -> &[World] {
        &self.worlds
    }

    fn index_of(&self, dimension: &str) -> Option<usize> {
        self.worlds.iter().position(|w| w.dimension() == dimension)
    }

    /// Runs at 20 TPS until `stop` or until no connection can reach it.
    pub fn run(mut self, inbox: Receiver<ToWorld>, stop: Arc<AtomicBool>) {
        let period = Duration::from_secs(1) / u32::from(TICK_RATE);
        let mut next = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if !self.tick(&inbox) {
                break;
            }
            next += period;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                next = now;
            }
        }
        for w in &mut self.worlds {
            w.stop();
        }
    }

    /// One tick of every dimension. Returns `false` once no connection can
    /// reach the node.
    pub fn tick(&mut self, inbox: &Receiver<ToWorld>) -> bool {
        let start = Instant::now();
        loop {
            match inbox.try_recv() {
                Ok(msg) => self.route(msg),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return false,
            }
        }
        for w in &mut self.worlds {
            w.step(start);
        }
        true
    }

    fn route(&mut self, msg: ToWorld) {
        match msg {
            ToWorld::Join { id, name, out } => {
                // A returning player joins the dimension it left in.
                let saved = players::load(&self.dir, &id).and_then(|s| s.dimension);
                let i = saved.and_then(|d| self.index_of(&d)).unwrap_or(0);
                if self.at.contains_key(&id) {
                    // The world kicks a second session of the same player.
                    let i = self.at[&id];
                    self.worlds[i].handle(ToWorld::Join { id, name, out });
                    return;
                }
                self.at.insert(id.clone(), i);
                self.worlds[i].handle(ToWorld::Join { id, name, out });
            }
            ToWorld::Leave { id } => {
                if let Some(i) = self.at.remove(&id) {
                    self.worlds[i].handle(ToWorld::Leave { id });
                }
            }
            ToWorld::Travel { id, dimension, pos } => {
                if let Err(e) = self.travel(&id, &dimension, pos) {
                    self.tell(&id, &e);
                }
            }
            ToWorld::Chat { id, text } => {
                let mut words = text.split_whitespace();
                if self.commands && words.next() == Some(TRAVEL_COMMAND) {
                    match words.next() {
                        Some(d) => {
                            if let Err(e) = self.travel(&id, d, None) {
                                self.tell(&id, &e);
                            }
                        }
                        None => {
                            let list = self.dimensions().join(", ");
                            self.tell(&id, &format!("dimensions: {list}"));
                        }
                    }
                }
            }
            msg => {
                let id = match &msg {
                    ToWorld::Intents { id, .. }
                    | ToWorld::Delivered { id, .. }
                    | ToWorld::Edit { id, .. } => id.clone(),
                    _ => return,
                };
                if let Some(&i) = self.at.get(&id) {
                    self.worlds[i].handle(msg);
                }
            }
        }
    }

    /// A message to one player, as a chat line from the server.
    fn tell(&self, id: &str, text: &str) {
        let mut end = text.len().min(protocol::MAX_CHAT_LEN);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if let Some(&i) = self.at.get(id) {
            self.worlds[i].send_to(
                id,
                ToConn::Event(protocol::ServerEvent::Chat {
                    from: String::new(),
                    text: text[..end].to_owned(),
                }),
            );
        }
    }

    /// Moves a player to `dimension`. The player must be spawned.
    pub fn travel(
        &mut self,
        id: &str,
        dimension: &str,
        pos: Option<[f64; 3]>,
    ) -> Result<(), String> {
        let Some(to) = self.index_of(dimension) else {
            return Err(format!(
                "no dimension {dimension:?} ({})",
                self.dimensions().join(", ")
            ));
        };
        let Some(&from) = self.at.get(id) else {
            return Err("not playing".into());
        };
        if from == to {
            return Err(format!("already in {dimension}"));
        }
        let Some(p) = self.worlds[from].take_player(id) else {
            return Err("not spawned yet".into());
        };
        self.worlds[to].insert_traveler(p, pos);
        self.at.insert(id.to_owned(), to);
        self.travels += 1;
        Ok(())
    }
}
