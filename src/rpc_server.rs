use anyhow::Result;
use futures::executor;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;
use tarpc::context;
use tokio::sync::mpsc::error::SendError;
use tracing::{error, info};
use twenty_first::shared_math::digest::Digest;
use twenty_first::util_types::algebraic_hasher::AlgebraicHasher;

use crate::config_models::network::Network;
use crate::models::blockchain::block::block_header::BlockHeader;
use crate::models::blockchain::block::block_height::BlockHeight;
use crate::models::blockchain::shared::Hash;
use crate::models::blockchain::transaction::amount::Amount;
use crate::models::blockchain::transaction::amount::Sign;
use crate::models::blockchain::transaction::utxo::Utxo;
use crate::models::channel::RPCServerToMain;
use crate::models::peer::PeerInfo;
use crate::models::state::wallet::address::generation_address;
use crate::models::state::wallet::wallet_status::WalletStatus;
use crate::models::state::{GlobalState, UtxoReceiverData};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashBoardOverviewDataFromClient {
    pub tip_header: BlockHeader,
    pub syncing: bool,
    pub synced_balance: Amount,
    pub mempool_size: usize,
    pub mempool_tx_count: usize,

    // `None` symbolizes failure in getting peer count
    pub peer_count: Option<usize>,

    // `None` symbolizes failure to get mining status
    pub is_mining: Option<bool>,

    // # of confirmations since last wallet balance change.
    // `None` indicates that wallet balance has never changed.
    pub confirmations: Option<BlockHeight>,
}

#[tarpc::service]
pub trait RPC {
    /******** READ DATA ********/
    // Place all methods that only read here
    // Return which network the client is running
    async fn get_network() -> Network;

    async fn get_listen_address_for_peers() -> Option<SocketAddr>;

    /// Returns the current block height.
    async fn block_height() -> BlockHeight;

    /// Returns the number of blocks (confirmations) since wallet balance last changed.
    ///
    /// returns `Option<BlockHeight>`
    ///
    /// return value will be None if wallet has not received any incoming funds.
    async fn get_confirmations() -> Option<BlockHeight>;

    /// Returns info about the peers we are connected to
    async fn get_peer_info() -> Vec<PeerInfo>;

    /// Returns the digest of the latest block
    async fn head() -> Digest;

    /// Returns the digest of the latest n blocks
    async fn heads(n: usize) -> Vec<Digest>;

    async fn get_tip_header() -> BlockHeader;

    async fn get_header(hash: Digest) -> Option<BlockHeader>;

    // Get sum of unspent UTXOs.
    async fn get_synced_balance() -> Amount;

    async fn get_history() -> Vec<(Digest, BlockHeight, Duration, Amount, Sign)>;

    async fn get_wallet_status() -> WalletStatus;

    async fn get_receiving_address() -> generation_address::ReceivingAddress;

    async fn get_mempool_tx_count() -> usize;

    // TODO: Change to return current size and max size
    async fn get_mempool_size() -> usize;

    async fn get_dashboard_overview_data() -> DashBoardOverviewDataFromClient;

    /// Determine whether the user-supplied string is a valid address
    async fn validate_address(
        address: String,
        network: Network,
    ) -> Option<generation_address::ReceivingAddress>;

    /// Determine whether the user-supplied string is a valid amount
    async fn validate_amount(amount: String) -> Option<Amount>;

    /// Determine whether the given amount is less than (or equal to) the balance
    async fn amount_leq_synced_balance(amount: Amount) -> bool;

    /******** CHANGE THINGS ********/
    // Place all things that change state here
    // Gracious shutdown.
    async fn shutdown() -> bool;

    /// Clears standing for all peers, connected or not
    async fn clear_all_standings();

    /// Clears standing for ip, whether connected or not
    async fn clear_ip_standing(ip: IpAddr);

    /// Send coins
    async fn send(
        amount: Amount,
        address: generation_address::ReceivingAddress,
        fee: Amount,
    ) -> Option<Digest>;

    // Stop miner if running
    async fn pause_miner();

    // Start miner if not running
    async fn restart_miner();

    // mark MUTXOs as abandoned
    async fn prune_abandoned_monitored_utxos() -> usize;
}

#[derive(Clone)]
pub struct NeptuneRPCServer {
    pub socket_address: SocketAddr,
    pub state: GlobalState,
    pub rpc_server_to_main_tx: tokio::sync::mpsc::Sender<RPCServerToMain>,
}

impl NeptuneRPCServer {
    fn confirmations_internal(&self) -> Option<BlockHeight> {

        match executor::block_on(self.state.get_latest_balance_height()) {
            Some(latest_balance_height) => {
                let tip_block_header =
                    executor::block_on(self.state.chain.light_state.header_partial());

                assert!(tip_block_header.height >= latest_balance_height);

                // subtract latest balance height from chain tip.
                // note: BlockHeight is u64 internally and BlockHeight::sub() returns i128.
                //       The subtraction and cast is safe given we passed the above assert.
                let confirmations: BlockHeight =
                    ((tip_block_header.height - latest_balance_height) as u64).into();
                Some(confirmations)
            }
            None => None,
        }
    }
}

impl RPC for NeptuneRPCServer {
    async fn get_network(self, _: context::Context) -> Network {
        self.state.cli.network
    }

    async fn get_listen_address_for_peers(self, _context: context::Context) -> Option<SocketAddr> {
        let listen_for_peers_ip = self.state.cli.listen_addr;
        let listen_for_peers_socket = self.state.cli.peer_port;
        let socket_address = SocketAddr::new(listen_for_peers_ip, listen_for_peers_socket);
        Some(socket_address)
    }

    async fn block_height(self, _: context::Context) -> BlockHeight {
        // let mut databases = executor::block_on(self.state.block_databases.lock());
        // let lookup_res = databases.latest_block_header.get(());
        let latest_block_header =
            executor::block_on(self.state.chain.light_state.header_clone());
        latest_block_header.height
    }

    async fn get_confirmations(self, _: context::Context) -> Option<BlockHeight> {
        self.confirmations_internal()
    }

    async fn head(self, _: context::Context) -> Digest {
        executor::block_on(self.state.chain.light_state.hash())
    }

    async fn heads(self, _context: tarpc::context::Context, n: usize) -> Vec<Digest> {
        let latest_block_digest =
            executor::block_on(self.state.chain.light_state.hash());

        let head_hashes = executor::block_on(
            self.state
                .chain
                .archival_state
                .as_ref()
                .expect("Can not give multiple ancestor hashes unless there is an archival state.")
                .get_ancestor_block_digests(latest_block_digest, n),
        );

        head_hashes
    }

    async fn get_peer_info(self, _: context::Context) -> Vec<PeerInfo> {
        let peer_map = self
            .state
            .net
            .peer_map
            .lock(|pm| pm.values().cloned().collect());
        peer_map
    }

    async fn validate_address(
        self,
        _ctx: context::Context,
        address_string: String,
        network: Network,
    ) -> Option<generation_address::ReceivingAddress> {
        let ret = if let Ok(address) =
            generation_address::ReceivingAddress::from_bech32m(address_string.clone(), network)
        {
            Some(address)
        } else {
            None
        };
        tracing::debug!(
            "Responding to address validation request of {address_string}: {}",
            ret.is_some()
        );
        ret
    }

    async fn validate_amount(
        self,
        _ctx: context::Context,
        amount_string: String,
    ) -> Option<Amount> {
        // parse string
        let amount = if let Ok(amt) = Amount::from_str(&amount_string) {
            amt
        } else {
            return None;
        };

        // return amount
        Some(amount)
    }

    async fn amount_leq_synced_balance(self, _ctx: context::Context, amount: Amount) -> bool {
        // test inequality
        let wallet_status = executor::block_on(self.state.get_wallet_status_for_tip());
        amount <= wallet_status.synced_unspent_amount
    }

    async fn get_synced_balance(self, _context: tarpc::context::Context) -> Amount {
        let wallet_status = executor::block_on(self.state.get_wallet_status_for_tip());
        wallet_status.synced_unspent_amount
    }

    async fn get_wallet_status(self, _context: tarpc::context::Context) -> WalletStatus {
        let wallet_status = executor::block_on(self.state.get_wallet_status_for_tip());
        wallet_status
    }

    async fn get_tip_header(self, _: context::Context) -> BlockHeader {
        let latest_block_block_header =
            executor::block_on(self.state.chain.light_state.header_clone());
        latest_block_block_header
    }

    async fn get_header(
        self,
        _context: tarpc::context::Context,
        block_digest: Digest,
    ) -> Option<BlockHeader> {
        let res = executor::block_on(
            self.state
                .chain
                .archival_state
                .as_ref()
                .expect("Can not give ancestor hash unless there is an archival state.")
                .get_block_header(block_digest),
        );
        res
    }

    async fn get_receiving_address(
        self,
        _context: tarpc::context::Context,
    ) -> generation_address::ReceivingAddress {
        self.state
            .wallet_state
            .wallet_secret
            .nth_generation_spending_key(0)
            .to_address()
    }

    async fn get_mempool_tx_count(self, _context: tarpc::context::Context) -> usize {
        self.state.mempool.len()
    }

    async fn get_mempool_size(self, _context: tarpc::context::Context) -> usize {
        self.state.mempool.get_size()
    }

    async fn get_history(
        self,
        _context: tarpc::context::Context,
    ) -> Vec<(Digest, BlockHeight, Duration, Amount, Sign)> {
        let history = executor::block_on(self.state.get_balance_history());

        // sort
        let mut display_history: Vec<(Digest, BlockHeight, Duration, Amount, Sign)> = history
            .iter()
            .map(|(h, t, bh, a, s)| (*h, *bh, *t, *a, *s))
            .collect::<Vec<_>>();
        display_history.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        // return
        display_history
    }

    async fn get_dashboard_overview_data(
        self,
        _context: tarpc::context::Context,
    ) -> DashBoardOverviewDataFromClient {
        let tip_header = executor::block_on(self.state.chain.light_state.header_clone());
        let wallet_status = executor::block_on(self.state.get_wallet_status_for_tip());
        let syncing = self.state.net.syncing.get();
        let mempool_size = self.state.mempool.get_size();
        let mempool_tx_count = self.state.mempool.len();

        // Return `None` if we fail to acquire the lock
        let peer_count = Some(self.state.net.peer_map.lock(|pm| pm.len()));

        let is_mining = Some(self.state.mining.lock(|m| *m));

        let confirmations = self.confirmations_internal();

        DashBoardOverviewDataFromClient {
            tip_header,
            syncing,
            synced_balance: wallet_status.synced_unspent_amount,
            mempool_size,
            mempool_tx_count,
            peer_count,
            is_mining,
            confirmations,
        }
    }

    // endpoints for changing stuff

    async fn clear_all_standings(self, _: context::Context) {
        self.state.net.peer_map.lock_mut(|pm| {
            pm.iter_mut().for_each(|(_, peerinfo)| {
                peerinfo.standing.clear_standing();
            });
        });

        // iterates and modifies standing field for all connected peers
        executor::block_on(self.state.clear_all_standings_in_database());
    }

    async fn clear_ip_standing(self, _: context::Context, ip: IpAddr) {
        self.state.net.peer_map.lock_mut(|pm| {
            pm.iter_mut().for_each(|(socketaddr, peerinfo)| {
                if socketaddr.ip() == ip {
                    peerinfo.standing.clear_standing();
                }
            });
        });
        //Also clears this IP's standing in database, whether it is connected or not.
        executor::block_on(self.state.clear_ip_standing_in_database(ip));
    }

    async fn send(
        self,
        _ctx: context::Context,
        amount: Amount,
        address: generation_address::ReceivingAddress,
        fee: Amount,
    ) -> Option<Digest> {
        let span = tracing::debug_span!("Constructing transaction objects");
        let _enter = span.enter();

        let coins = amount.to_native_coins();
        let utxo = Utxo::new(address.lock_script(), coins);
        let block_height = executor::block_on(self.state.chain.light_state.header_partial()).height;
        let receiver_privacy_digest = address.privacy_digest;
        let sender_randomness = self
            .state
            .wallet_state
            .wallet_secret
            .generate_sender_randomness(block_height, receiver_privacy_digest);

        // 1. Build transaction object
        // TODO: Allow user to set fee here. Don't set it automatically as we want the user
        // to be in control of this. But we could add an endpoint to get recommended fee
        // density.
        let (pubscript, pubscript_input) =
            match address.generate_pubscript_and_input(&utxo, sender_randomness) {
                Ok((ps, inp)) => (ps, inp),
                Err(_) => {
                    tracing::error!(
                        "Failed to generate transaction because could not encrypt to address."
                    );
                    return None;
                }
            };
        let receiver_data = [(UtxoReceiverData {
            utxo,
            sender_randomness,
            receiver_privacy_digest,
            pubscript,
            pubscript_input,
        })]
        .to_vec();

        // Pause miner if we are mining
        let was_mining = self.state.mining.lock(|m| *m);
        if was_mining {
            let _ =
                executor::block_on(self.rpc_server_to_main_tx.send(RPCServerToMain::PauseMiner));
        }

        let transaction_result =
            executor::block_on(self.state.create_transaction(receiver_data, fee));

        let transaction = match transaction_result {
            Ok(tx) => tx,
            Err(err) => panic!("Could not create transaction: {}", err),
        };

        // 2. Send transaction message to main
        let response: Result<(), SendError<RPCServerToMain>> = executor::block_on(
            self.rpc_server_to_main_tx
                .send(RPCServerToMain::Send(Box::new(transaction.clone()))),
        );

        // Restart mining if it was paused
        if was_mining {
            let _ = executor::block_on(
                self.rpc_server_to_main_tx
                    .send(RPCServerToMain::RestartMiner),
            );
        }

        if response.is_ok() {
            Some(Hash::hash(&transaction))
        } else {
            None
        }
    }

    async fn shutdown(self, _: context::Context) -> bool {
        // 1. Send shutdown message to main
        let response =
            executor::block_on(self.rpc_server_to_main_tx.send(RPCServerToMain::Shutdown));

        // 2. Send acknowledgement to client.
        response.is_ok()
    }

    async fn pause_miner(self, _context: tarpc::context::Context) {
        if self.state.cli.mine {
            let _ =
                executor::block_on(self.rpc_server_to_main_tx.send(RPCServerToMain::PauseMiner));
        } else {
            info!("Cannot pause miner since it was never started");
        }
    }

    async fn restart_miner(self, _context: tarpc::context::Context) {
        if self.state.cli.mine {
            let _ = executor::block_on(
                self.rpc_server_to_main_tx
                    .send(RPCServerToMain::RestartMiner),
            );
        } else {
            info!("Cannot restart miner since it was never started");
        }
    }

    async fn prune_abandoned_monitored_utxos(
        self,
        _context: tarpc::context::Context,
    ) -> usize {
        let prune_count_res = executor::block_on(async move {
            let tip_block_header = self.state.chain.light_state.header_clone().await;
            const DEFAULT_MUTXO_PRUNE_DEPTH: usize = 200;

            self.state
                .wallet_state
                .prune_abandoned_monitored_utxos_with_lock(
                    DEFAULT_MUTXO_PRUNE_DEPTH,
                    &tip_block_header,
                    &self.state.chain.archival_state.unwrap(),
                )
                .await
        });

        match prune_count_res {
            Ok(prune_count) => {
                info!("Marked {prune_count} monitored UTXOs as abandoned");
                prune_count
            }
            Err(err) => {
                error!("Pruning monitored UTXOs failed with error: {err}");
                0
            }
        }
    }
}

#[cfg(test)]
mod rpc_server_tests {
    use super::*;
    use crate::{
        config_models::network::Network,
        models::{
            peer::PeerSanctionReason,
            state::wallet::{generate_secret_key, WalletSecret},
        },
        rpc_server::NeptuneRPCServer,
        tests::shared::{get_mock_global_state, get_test_genesis_setup},
        RPC_CHANNEL_CAPACITY,
    };
    use anyhow::Result;
    use num_traits::Zero;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use tracing_test::traced_test;

    #[traced_test]
    #[tokio::test]
    async fn balance_is_zero_at_init() -> Result<()> {
        // Verify that a wallet not receiving a premine is empty at startup
        let network = Network::Alpha;
        let state =
            get_mock_global_state(network, 2, Some(WalletSecret::new(generate_secret_key()))).await;
        let (dummy_tx, _rx) = tokio::sync::mpsc::channel::<RPCServerToMain>(RPC_CHANNEL_CAPACITY);
        let rpc_server = NeptuneRPCServer {
            socket_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080),
            state: state.clone(),
            rpc_server_to_main_tx: dummy_tx,
        };

        let balance = rpc_server.get_synced_balance(context::current()).await;
        assert!(balance.is_zero());

        Ok(())
    }

    #[traced_test]
    #[tokio::test]
    async fn clear_ip_standing_test() -> Result<()> {
        // Create initial conditions
        let (_peer_broadcast_tx, _from_main_rx_clone, _to_main_tx, mut _to_main_rx, state, _hsd) =
            get_test_genesis_setup(Network::Alpha, 2).await?;
        let peer_address_0 =
            state.net.peer_map.lock_guard().values().collect::<Vec<_>>()[0].connected_address;
        let peer_address_1 =
            state.net.peer_map.lock_guard().values().collect::<Vec<_>>()[1].connected_address;

        // sanction both
        let (standing_0, standing_1) = {
            let mut peers = state.net.peer_map.lock_guard_mut();
            peers.entry(peer_address_0).and_modify(|p| {
                p.standing.sanction(PeerSanctionReason::DifferentGenesis);
            });
            peers.entry(peer_address_1).and_modify(|p| {
                p.standing.sanction(PeerSanctionReason::DifferentGenesis);
            });
            let standing_0 = peers[&peer_address_0].standing;
            let standing_1 = peers[&peer_address_1].standing;
            (standing_0, standing_1)
        };

        state
            .write_peer_standing_on_decrease(peer_address_0.ip(), standing_0)
            .await;
        state
            .write_peer_standing_on_decrease(peer_address_1.ip(), standing_1)
            .await;

        // Verify expected initial conditions
        {
            let peer_standing_0 = state
                .get_peer_standing_from_database(peer_address_0.ip())
                .await;
            assert_ne!(0, peer_standing_0.unwrap().standing);
            assert_ne!(None, peer_standing_0.unwrap().latest_sanction);
            let peer_standing_1 = state
                .get_peer_standing_from_database(peer_address_1.ip())
                .await;
            assert_ne!(0, peer_standing_1.unwrap().standing);
            assert_ne!(None, peer_standing_1.unwrap().latest_sanction);

            // Clear standing of #0
            let (dummy_tx, _rx) =
                tokio::sync::mpsc::channel::<RPCServerToMain>(RPC_CHANNEL_CAPACITY);
            let rpc_server = NeptuneRPCServer {
                socket_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080),
                state: state.clone(),
                rpc_server_to_main_tx: dummy_tx,
            };
            rpc_server
                .clear_ip_standing(context::current(), peer_address_0.ip())
                .await;
        }
        // Verify expected resulting conditions in database
        {
            let peer_standing_0 = state
                .get_peer_standing_from_database(peer_address_0.ip())
                .await;
            assert_eq!(0, peer_standing_0.unwrap().standing);
            assert_eq!(None, peer_standing_0.unwrap().latest_sanction);
            let peer_standing_1 = state
                .get_peer_standing_from_database(peer_address_1.ip())
                .await;
            assert_ne!(0, peer_standing_1.unwrap().standing);
            assert_ne!(None, peer_standing_1.unwrap().latest_sanction);

            // Verify expected resulting conditions in peer map
            let peer_standing_0_from_memory =
                state.net.peer_map.lock_guard()[&peer_address_0].clone();
            assert_eq!(0, peer_standing_0_from_memory.standing.standing);
            let peer_standing_1_from_memory =
                state.net.peer_map.lock_guard()[&peer_address_1].clone();
            assert_ne!(0, peer_standing_1_from_memory.standing.standing);
        }
        Ok(())
    }
    #[traced_test]
    #[tokio::test]
    async fn clear_all_standings_test() -> Result<()> {
        // Create initial conditions
        let (_peer_broadcast_tx, _from_main_rx_clone, _to_main_tx, mut _to_main_rx, state, _hsd) =
            get_test_genesis_setup(Network::Alpha, 2).await?;
        let peer_address_0 =
            state.net.peer_map.lock_guard().values().collect::<Vec<_>>()[0].connected_address;
        let peer_address_1 =
            state.net.peer_map.lock_guard().values().collect::<Vec<_>>()[1].connected_address;

        // sanction both peers
        let (standing_0, standing_1) = {
            let mut peers = state.net.peer_map.lock_guard_mut();

            peers.entry(peer_address_0).and_modify(|p| {
                p.standing.sanction(PeerSanctionReason::DifferentGenesis);
            });
            peers.entry(peer_address_1).and_modify(|p| {
                p.standing.sanction(PeerSanctionReason::DifferentGenesis);
            });
            let standing_0 = peers[&peer_address_0].standing;
            let standing_1 = peers[&peer_address_1].standing;
            (standing_0, standing_1)
        };

        state
            .write_peer_standing_on_decrease(peer_address_0.ip(), standing_0)
            .await;
        state
            .write_peer_standing_on_decrease(peer_address_1.ip(), standing_1)
            .await;

        // Verify expected initial conditions
        {
            let peer_standing_0 = state
                .get_peer_standing_from_database(peer_address_0.ip())
                .await;
            assert_ne!(0, peer_standing_0.unwrap().standing);
            assert_ne!(None, peer_standing_0.unwrap().latest_sanction);
        }

        {
            let peer_standing_1 = state
                .get_peer_standing_from_database(peer_address_1.ip())
                .await;
            assert_ne!(0, peer_standing_1.unwrap().standing);
            assert_ne!(None, peer_standing_1.unwrap().latest_sanction);
        }

        // Clear standing of both by clearing all standings
        let (dummy_tx, _rx) = tokio::sync::mpsc::channel::<RPCServerToMain>(RPC_CHANNEL_CAPACITY);
        let rpc_server = NeptuneRPCServer {
            socket_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080),
            state: state.clone(),
            rpc_server_to_main_tx: dummy_tx.clone(),
        };
        rpc_server.clear_all_standings(context::current()).await;

        // Verify expected resulting conditions in database
        {
            let peer_standing_0 = state
                .get_peer_standing_from_database(peer_address_0.ip())
                .await;
            assert_eq!(0, peer_standing_0.unwrap().standing);
            assert_eq!(None, peer_standing_0.unwrap().latest_sanction);
        }

        {
            let peer_still_standing_1 = state
                .get_peer_standing_from_database(peer_address_1.ip())
                .await;
            assert_eq!(0, peer_still_standing_1.unwrap().standing);
            assert_eq!(None, peer_still_standing_1.unwrap().latest_sanction);
        }

        // Verify expected resulting conditions in peer map
        {
            let peer_standing_0_from_memory =
                state.net.peer_map.lock_guard()[&peer_address_0].clone();
            assert_eq!(0, peer_standing_0_from_memory.standing.standing);
        }

        {
            let peer_still_standing_1_from_memory =
                state.net.peer_map.lock_guard()[&peer_address_1].clone();
            assert_eq!(0, peer_still_standing_1_from_memory.standing.standing);
        }

        Ok(())
    }
}
