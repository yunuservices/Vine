pub mod bridge;
mod bungee;
mod client_queue;
mod connection;
pub mod forwarding;
mod login;
mod readers;
mod suggestions;
mod switch;
mod task;
mod tunnel;

pub use bridge::{BackendBridge, BridgeError};

use crate::config::{BackendConfig, RoutingConfig};
use std::collections::HashMap;

pub struct Router {
    routing: RoutingConfig,
    servers: HashMap<String, BackendConfig>,
}

impl Router {
    pub fn new(routing: RoutingConfig, servers: HashMap<String, BackendConfig>) -> Self {
        Self { routing, servers }
    }

    /// Servers to try, in order, for a player who connected with this hostname
    pub fn servers_for(&self, virtual_host: &str) -> Vec<(String, BackendConfig)> {
        self.routing
            .servers_for(virtual_host)
            .iter()
            .filter_map(|name| Some((name.clone(), self.servers.get(name)?.clone())))
            .collect()
    }
}
