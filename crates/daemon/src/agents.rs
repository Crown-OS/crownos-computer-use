//! Who is on each connection, so every action has a name and a cursor.

use std::collections::HashMap;

use crownos_ipc::PeerId;
use crownos_window_layer::CursorId;

use crate::util::rate::RateLimiter;

pub(crate) const ACTIONS_PER_SECOND: u32 = 20;

#[derive(Debug)]
pub(crate) struct Agent {
    pub(crate) name: String,
    pub(crate) cursor: Option<CursorId>,
    pub(crate) limiter: RateLimiter,
}

#[derive(Debug, Default)]
pub(crate) struct Agents {
    next_id: u32,
    by_peer: HashMap<PeerId, Agent>,
}

impl Agents {
    pub(crate) fn register(&mut self, peer: PeerId, name: String, cursor: Option<CursorId>) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        self.by_peer.insert(
            peer,
            Agent {
                name,
                cursor,
                limiter: RateLimiter::per_second(ACTIONS_PER_SECOND),
            },
        );
        id
    }

    pub(crate) fn get_mut(&mut self, peer: PeerId) -> Option<&mut Agent> {
        self.by_peer.get_mut(&peer)
    }

    pub(crate) fn name(&self, peer: PeerId) -> &str {
        self.by_peer
            .get(&peer)
            .map_or("anonymous", |agent| agent.name.as_str())
    }

    pub(crate) fn remove(&mut self, peer: PeerId) -> Option<Agent> {
        self.by_peer.remove(&peer)
    }
}
