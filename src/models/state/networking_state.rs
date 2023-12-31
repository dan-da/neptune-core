use crate::config_models::data_directory::DataDirectory;
use crate::database::rusty::{default_options, RustyLevelDbAsync};
use crate::models::database::PeerDatabases;
use crate::models::peer::{self, PeerStanding};
use anyhow::Result;
use std::net::IpAddr;
use std::{collections::HashMap, net::SocketAddr};
use twenty_first::sync;

pub const BANNED_IPS_DB_NAME: &str = "banned_ips";

type PeerMap = HashMap<SocketAddr, peer::PeerInfo>;

/// `NetworkingState` contains in-memory and persisted data for interacting
/// with network peers.
#[derive(Debug, Clone)]
pub struct NetworkingState {
    // Stores info about the peers that the client is connected to
    // Peer threads may update their own entries into this map.
    pub peer_map: sync::AtomicRw<PeerMap>,

    // `peer_databases` are used to persist IPs with their standing.
    // The peer threads may update their own entries into this map.
    pub peer_databases: PeerDatabases,

    // This value is only true if instance is running an archival node
    // that is currently downloading blocks to catch up.
    // Only the main thread may update this flag
    pub syncing: sync::AtomicRw<bool>,

    // Read-only value set during startup
    pub instance_id: u128,
}

impl NetworkingState {
    pub fn new(peer_map: PeerMap, peer_databases: PeerDatabases, syncing: bool) -> Self {
        Self {
            peer_map: sync::AtomicRw::from((
                peer_map,
                Some("NetworkingState::peer_map"),
                Some(crate::LOG_LOCK_EVENT_CB),
            )),
            peer_databases,
            syncing: sync::AtomicRw::from((
                syncing,
                Some("NetworkingState::syncing"),
                Some(crate::LOG_LOCK_EVENT_CB),
            )),
            instance_id: rand::random(),
        }
    }

    /// Create databases for peer standings
    pub async fn initialize_peer_databases(data_dir: &DataDirectory) -> Result<PeerDatabases> {
        let database_dir_path = data_dir.database_dir_path();
        DataDirectory::create_dir_if_not_exists(&database_dir_path)?;

        let peer_standings = RustyLevelDbAsync::<IpAddr, PeerStanding>::new(
            &data_dir.banned_ips_database_dir_path(),
            default_options(),
        )
        .await?;

        Ok(PeerDatabases { peer_standings })
    }
}
