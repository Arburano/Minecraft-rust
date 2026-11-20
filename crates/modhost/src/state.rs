use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use world::{BlockStateId, SECTION_SIZE, Section};

use crate::bindings::engine::api::{components, host_log, types, world_access};
use crate::{ApiError, BlockChange, BlockPos, Error};

const MAX_WRITES: usize = 16_384;
const MAX_COMPONENTS: usize = 64;
const MAX_COMPONENT_BYTES: usize = 1024 * 1024;

/// Region-local state for the spike, backed by one 16³ world section.
pub struct Region {
    origin: BlockPos,
    section: Section,
    state_ids: BTreeMap<String, u16>,
    known_states: BTreeSet<u16>,
    component_names: Vec<(String, bool)>,
    component_values: BTreeMap<(u32, u64), Vec<u8>>,
    messages: Vec<(BlockPos, String, Vec<u8>)>,
}

impl Region {
    /// Creates an empty region with caller-supplied registry entries; ID 0 is air.
    pub fn new(origin: BlockPos, states: impl IntoIterator<Item = (String, BlockStateId)>) -> Self {
        let state_ids: BTreeMap<_, _> = states.into_iter().map(|(name, id)| (name, id.0)).collect();
        let mut known_states: BTreeSet<_> = state_ids.values().copied().collect();
        known_states.insert(0);
        Self {
            origin,
            section: Section::new(),
            state_ids,
            known_states,
            component_names: Vec::with_capacity(MAX_COMPONENTS),
            component_values: BTreeMap::new(),
            messages: Vec::new(),
        }
    }

    /// Reads a committed block or returns an out-of-region error.
    pub fn get_block(&self, pos: BlockPos) -> Result<BlockStateId, ApiError> {
        self.index(pos).map(|index| self.section.get(index))
    }

    /// Returns the committed section for reference comparisons and tick work.
    pub fn section(&self) -> &Section {
        &self.section
    }

    /// Returns host-owned component state after a successful hook.
    pub fn component(&self, component: u32, entity: u64) -> Option<&[u8]> {
        self.component_values
            .get(&(component, entity))
            .map(Vec::as_slice)
    }

    /// Takes committed cross-region messages for delivery on the following tick.
    pub fn take_messages(&mut self) -> Vec<(BlockPos, String, Vec<u8>)> {
        std::mem::take(&mut self.messages)
    }

    fn index(&self, pos: BlockPos) -> Result<usize, ApiError> {
        let x = i64::from(pos.x) - i64::from(self.origin.x);
        let y = i64::from(pos.y) - i64::from(self.origin.y);
        let z = i64::from(pos.z) - i64::from(self.origin.z);
        let bounds = 0..i64::from(SECTION_SIZE);
        if !bounds.contains(&x) || !bounds.contains(&y) || !bounds.contains(&z) {
            return Err(ApiError::OutOfRegion(pos));
        }
        Ok((x + 16 * z + 256 * y) as usize)
    }
}

pub(crate) struct HostState {
    pub(crate) region: Region,
    pub(crate) deadline: Instant,
    pub(crate) interrupt_at: Option<Instant>,
    pub(crate) returned_at: Option<Instant>,
    writes: Vec<(usize, BlockStateId)>,
    component_writes: Vec<(u32, u64, Vec<u8>)>,
    registrations: Vec<(String, bool)>,
    messages: Vec<(BlockPos, String, Vec<u8>)>,
    staged_bytes: usize,
    failure: Option<ApiError>,
}

impl HostState {
    pub(crate) fn new(region: Region) -> Self {
        Self {
            region,
            deadline: Instant::now(),
            interrupt_at: None,
            returned_at: None,
            writes: Vec::with_capacity(MAX_WRITES),
            component_writes: Vec::with_capacity(MAX_WRITES),
            registrations: Vec::with_capacity(MAX_COMPONENTS),
            messages: Vec::with_capacity(MAX_WRITES),
            staged_bytes: 0,
            failure: None,
        }
    }

    pub(crate) fn begin(&mut self, deadline: Instant) {
        self.discard();
        self.deadline = deadline;
        self.interrupt_at = None;
        self.returned_at = None;
    }

    pub(crate) fn pending_blocks(&self) -> usize {
        self.writes.len()
    }

    pub(crate) fn discard(&mut self) {
        self.writes.clear();
        self.component_writes.clear();
        self.registrations.clear();
        self.messages.clear();
        self.staged_bytes = 0;
        self.failure = None;
    }

    pub(crate) fn commit(&mut self) -> Result<(), Error> {
        if let Some(error) = self.failure.take() {
            self.discard();
            return Err(Error::Transaction(error));
        }
        for (index, state) in self.writes.drain(..) {
            self.region.section.set(index, state);
        }
        self.region.component_names.append(&mut self.registrations);
        for (component, entity, bytes) in self.component_writes.drain(..) {
            self.region
                .component_values
                .insert((component, entity), bytes);
        }
        self.region.messages.append(&mut self.messages);
        self.staged_bytes = 0;
        Ok(())
    }

    fn check_deadline(&self) -> wasmtime::Result<()> {
        if Instant::now() >= self.deadline {
            return Err(wasmtime::Error::msg(
                "mod hard deadline exceeded in host import",
            ));
        }
        Ok(())
    }

    fn reject<T>(&mut self, error: ApiError) -> Result<T, ApiError> {
        self.failure = Some(error.clone());
        Err(error)
    }
}

impl types::Host for HostState {}

impl world_access::Host for HostState {
    fn get_blocks(
        &mut self,
        positions: Vec<BlockPos>,
    ) -> wasmtime::Result<Result<Vec<u16>, ApiError>> {
        self.check_deadline()?;
        if positions.len() > MAX_WRITES {
            return Ok(Err(ApiError::BudgetExceeded));
        }
        let mut states = Vec::with_capacity(positions.len());
        for pos in positions {
            self.check_deadline()?;
            let index = match self.region.index(pos) {
                Ok(index) => index,
                Err(error) => return Ok(Err(error)),
            };
            states.push(
                self.writes
                    .iter()
                    .rev()
                    .find(|(i, _)| *i == index)
                    .map_or_else(|| self.region.section.get(index).0, |(_, state)| state.0),
            );
        }
        Ok(Ok(states))
    }

    fn set_blocks(&mut self, changes: Vec<BlockChange>) -> wasmtime::Result<Result<(), ApiError>> {
        self.check_deadline()?;
        if changes.len() > MAX_WRITES - self.writes.len() {
            return Ok(self.reject(ApiError::BudgetExceeded));
        }
        for change in &changes {
            if let Err(error) = self.region.index(change.pos) {
                return Ok(self.reject(error));
            }
            if !self.region.known_states.contains(&change.state) {
                return Ok(self.reject(ApiError::UnknownId(change.state.to_string())));
            }
        }
        for change in changes {
            let index = self
                .region
                .index(change.pos)
                .map_err(|error| wasmtime::Error::msg(format!("{error:?}")))?;
            self.writes.push((index, BlockStateId(change.state)));
        }
        self.check_deadline()?;
        Ok(Ok(()))
    }

    fn state_id(&mut self, name: String) -> wasmtime::Result<Result<u16, ApiError>> {
        self.check_deadline()?;
        Ok(self
            .region
            .state_ids
            .get(&name)
            .copied()
            .ok_or(ApiError::UnknownId(name)))
    }

    fn region_contains(&mut self, pos: BlockPos) -> wasmtime::Result<bool> {
        self.check_deadline()?;
        Ok(self.region.index(pos).is_ok())
    }

    fn send_to(
        &mut self,
        pos: BlockPos,
        channel: String,
        payload: Vec<u8>,
    ) -> wasmtime::Result<()> {
        self.check_deadline()?;
        let bytes = channel.len().saturating_add(payload.len());
        if self.messages.len() >= MAX_WRITES || bytes > MAX_COMPONENT_BYTES - self.staged_bytes {
            self.failure = Some(ApiError::BudgetExceeded);
        } else {
            self.staged_bytes += bytes;
            self.messages.push((pos, channel, payload));
        }
        Ok(())
    }
}

impl components::Host for HostState {
    fn register(
        &mut self,
        name: String,
        replicate: bool,
    ) -> wasmtime::Result<Result<u32, ApiError>> {
        self.check_deadline()?;
        let names = self
            .region
            .component_names
            .iter()
            .chain(&self.registrations);
        if let Some(index) = names.clone().position(|(n, _)| *n == name) {
            return Ok(Ok(index as u32));
        }
        let index = self.region.component_names.len() + self.registrations.len();
        if index >= MAX_COMPONENTS || name.len() > 256 {
            return Ok(self.reject(ApiError::BudgetExceeded));
        }
        self.registrations.push((name, replicate));
        Ok(Ok(index as u32))
    }

    fn get(
        &mut self,
        component: u32,
        entities: Vec<u64>,
    ) -> wasmtime::Result<Vec<Option<Vec<u8>>>> {
        self.check_deadline()?;
        if entities.len() > MAX_WRITES {
            return Err(wasmtime::Error::msg(
                "component read batch exceeds the spike limit",
            ));
        }
        let mut values = Vec::with_capacity(entities.len());
        let mut total_bytes = 0;
        for entity in entities {
            self.check_deadline()?;
            let bytes = self
                .component_writes
                .iter()
                .rev()
                .find(|(c, e, _)| *c == component && *e == entity)
                .map(|(_, _, bytes)| bytes)
                .or_else(|| self.region.component_values.get(&(component, entity)));
            if let Some(bytes) = bytes {
                if bytes.len() > MAX_COMPONENT_BYTES - total_bytes {
                    return Err(wasmtime::Error::msg(
                        "component read payload exceeds the spike limit",
                    ));
                }
                total_bytes += bytes.len();
            }
            values.push(bytes.cloned());
        }
        Ok(values)
    }

    fn set(
        &mut self,
        component: u32,
        values: Vec<(u64, Vec<u8>)>,
    ) -> wasmtime::Result<Result<(), ApiError>> {
        self.check_deadline()?;
        if component as usize >= self.region.component_names.len() + self.registrations.len() {
            return Ok(self.reject(ApiError::UnknownId(component.to_string())));
        }
        let bytes = values
            .iter()
            .fold(0usize, |sum, (_, value)| sum.saturating_add(value.len()));
        if values.len() > MAX_WRITES - self.component_writes.len()
            || bytes > MAX_COMPONENT_BYTES - self.staged_bytes
        {
            return Ok(self.reject(ApiError::BudgetExceeded));
        }
        self.staged_bytes += bytes;
        self.component_writes.extend(
            values
                .into_iter()
                .map(|(entity, bytes)| (component, entity, bytes)),
        );
        Ok(Ok(()))
    }

    fn query(&mut self, component: u32) -> wasmtime::Result<Vec<u64>> {
        self.check_deadline()?;
        let mut entities = BTreeSet::new();
        for (c, e) in self
            .region
            .component_values
            .keys()
            .copied()
            .chain(self.component_writes.iter().map(|(c, e, _)| (*c, *e)))
        {
            self.check_deadline()?;
            if c == component {
                entities.insert(e);
                if entities.len() > MAX_WRITES {
                    return Err(wasmtime::Error::msg(
                        "component query exceeds the spike limit",
                    ));
                }
            }
        }
        Ok(entities.into_iter().collect())
    }
}

impl host_log::Host for HostState {
    fn log(&mut self, _level: host_log::Level, _message: String) -> wasmtime::Result<()> {
        self.check_deadline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> HostState {
        let mut state = HostState::new(Region::new(
            BlockPos { x: 0, y: 0, z: 0 },
            [("test:block".into(), BlockStateId(1))],
        ));
        state.begin(Instant::now() + std::time::Duration::from_secs(10));
        state
    }

    #[test]
    fn coordinate_bounds_do_not_overflow() {
        let region = Region::new(
            BlockPos {
                x: i32::MIN,
                y: 0,
                z: 0,
            },
            [],
        );
        assert!(
            region
                .index(BlockPos {
                    x: i32::MAX,
                    y: 0,
                    z: 0
                })
                .is_err()
        );
        assert_eq!(
            region
                .index(BlockPos {
                    x: i32::MIN + 15,
                    y: 15,
                    z: 15
                })
                .expect("inside"),
            4095
        );
    }

    #[test]
    fn reads_see_staged_changes_but_region_does_not() -> wasmtime::Result<()> {
        use world_access::Host;
        let mut state = state();
        let pos = BlockPos { x: 0, y: 0, z: 0 };
        assert!(
            state
                .set_blocks(vec![BlockChange { pos, state: 1 }])?
                .is_ok()
        );
        assert_eq!(state.get_blocks(vec![pos])?.expect("valid read"), vec![1]);
        assert_eq!(
            state.region.get_block(pos).expect("inside"),
            BlockStateId::AIR
        );
        state.discard();
        assert_eq!(state.region.section.non_air(), 0);
        Ok(())
    }

    #[test]
    fn unknown_state_invalidates_whole_transaction() -> wasmtime::Result<()> {
        use world_access::Host;
        let mut state = state();
        let pos = BlockPos { x: 0, y: 0, z: 0 };
        assert!(
            state
                .set_blocks(vec![BlockChange { pos, state: 1 }])?
                .is_ok()
        );
        assert!(
            state
                .set_blocks(vec![BlockChange { pos, state: 2 }])?
                .is_err()
        );
        assert!(state.commit().is_err());
        assert_eq!(state.region.section.non_air(), 0);
        Ok(())
    }

    #[test]
    fn components_and_messages_commit_or_discard_together() -> wasmtime::Result<()> {
        let mut state = state();
        let component = components::Host::register(&mut state, "test:data".into(), false)?
            .expect("registration");
        assert!(
            components::Host::set(&mut state, component, vec![(3, vec![1, 2]), (1, vec![3])])?
                .is_ok()
        );
        assert_eq!(components::Host::query(&mut state, component)?, vec![1, 3]);
        assert_eq!(
            components::Host::get(&mut state, component, vec![3, 9])?,
            vec![Some(vec![1, 2]), None]
        );
        world_access::Host::send_to(
            &mut state,
            BlockPos { x: 16, y: 0, z: 0 },
            "test:message".into(),
            vec![7],
        )?;
        assert!(state.region.component(component, 3).is_none());
        state.commit().expect("valid transaction");
        assert_eq!(
            state.region.component(component, 3),
            Some([1, 2].as_slice())
        );
        assert_eq!(state.region.take_messages().len(), 1);
        assert!(state.region.take_messages().is_empty());
        assert!(components::Host::set(&mut state, component, vec![(3, vec![9])])?.is_ok());
        world_access::Host::send_to(
            &mut state,
            BlockPos { x: 16, y: 0, z: 0 },
            "test:discard".into(),
            vec![0],
        )?;
        state.discard();
        assert_eq!(
            state.region.component(component, 3),
            Some([1, 2].as_slice())
        );
        assert!(state.region.take_messages().is_empty());
        Ok(())
    }

    #[test]
    fn registry_and_region_checks_are_explicit() -> wasmtime::Result<()> {
        use world_access::Host;
        let mut state = state();
        assert_eq!(state.state_id("test:block".into())?.expect("registered"), 1);
        assert!(state.state_id("test:unknown".into())?.is_err());
        assert!(state.region_contains(BlockPos {
            x: 15,
            y: 15,
            z: 15
        })?);
        assert!(!state.region_contains(BlockPos { x: -1, y: 0, z: 0 })?);
        assert!(
            state
                .get_blocks(vec![BlockPos { x: 0, y: 16, z: 0 }])?
                .is_err()
        );
        state.deadline = Instant::now();
        assert!(state.get_blocks(Vec::new()).is_err());
        Ok(())
    }

    #[test]
    fn write_limit_discards_batch() -> wasmtime::Result<()> {
        use world_access::Host;
        let mut state = state();
        let changes = vec![
            BlockChange {
                pos: BlockPos { x: 0, y: 0, z: 0 },
                state: 1
            };
            MAX_WRITES + 1
        ];
        assert!(state.set_blocks(changes)?.is_err());
        assert!(state.commit().is_err());
        assert_eq!(state.region.section.non_air(), 0);
        Ok(())
    }
}
