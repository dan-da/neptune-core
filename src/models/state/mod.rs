use self::blockchain_state::BlockchainState;
use self::mempool::Mempool;
use self::networking_state::NetworkingState;
use self::wallet::address::ReceivingAddressType;
use self::wallet::address::SpendingKeyType;
use self::wallet::utxo_notification_pool::UtxoNotifier;
use self::wallet::wallet_state::WalletState;
use self::wallet::wallet_status::WalletStatus;
use super::blockchain::block::block_height::BlockHeight;
use super::blockchain::block::Block;
use super::blockchain::transaction::primitive_witness::{PrimitiveWitness, SaltedUtxos};
use super::blockchain::transaction::transaction_kernel::TransactionKernel;
use super::blockchain::transaction::utxo::Utxo;
use super::blockchain::transaction::validity::TransactionValidationLogic;
use super::blockchain::transaction::Transaction;
use super::blockchain::transaction::TxInputList;
use super::blockchain::transaction::TxOutput;
use super::blockchain::transaction::TxOutputList;
use super::blockchain::transaction::UtxoNotifyMethod;
use super::blockchain::type_scripts::native_currency::NativeCurrency;
use super::blockchain::type_scripts::neptune_coins::NeptuneCoins;
use super::blockchain::type_scripts::time_lock::TimeLock;
use super::blockchain::type_scripts::TypeScript;
use super::consensus::tasm::program::ConsensusProgram;
use super::consensus::timestamp::Timestamp;
use crate::config_models::cli_args;
use crate::database::storage::storage_schema::traits::StorageWriter as SW;
use crate::database::storage::storage_vec::traits::*;
use crate::database::storage::storage_vec::Index;
use crate::locks::tokio as sync_tokio;
use crate::models::peer::HandshakeData;
use crate::models::state::wallet::monitored_utxo::MonitoredUtxo;
use crate::models::state::wallet::utxo_notification_pool::ExpectedUtxo;
use crate::prelude::twenty_first;
use crate::time_fn_call_async;
use crate::util_types::mutator_set::mutator_set_accumulator::MutatorSetAccumulator;
use crate::{Hash, VERSION};
use anyhow::{bail, Result};
use itertools::Itertools;
use num_traits::CheckedSub;
use std::cmp::max;
use std::ops::{Deref, DerefMut};
use tracing::{debug, info, warn};
use twenty_first::math::digest::Digest;
use twenty_first::util_types::algebraic_hasher::AlgebraicHasher;

pub mod archival_state;
pub mod blockchain_state;
pub mod light_state;
pub mod mempool;
pub mod networking_state;
pub mod shared;
pub mod wallet;

#[derive(Debug, Clone)]
struct TransactionDetails {
    pub tx_inputs: TxInputList,
    pub tx_outputs: TxOutputList,
    pub fee: NeptuneCoins,
    pub timestamp: Timestamp,
}

/// `GlobalStateLock` holds a [`tokio::AtomicRw`](crate::locks::tokio::AtomicRw)
/// ([`RwLock`](std::sync::RwLock)) over [`GlobalState`].
///
/// Conceptually** all reads and writes of application state
/// require acquiring this lock.
///
/// Having a single lock is useful for a few reasons:
///  1. Enables write serialization over all application state.
///     (blockchain, mempool, wallet, global flags)
///  2. Readers see a consistent view of data.
///  3. makes it easy to reason about locking.
///  4. simplifies the codebase.
///
/// The primary drawback is that long write operations can
/// block readers.  As such, every effort should be made to keep
/// write operations as short as possible, though
/// correctness/atomicity have first priority.
///
/// Using an `RwLock` is beneficial for concurrency vs using a `Mutex`.
/// Readers do not block eachother.  Only a writer blocks readers.
/// See [`RwLock`](std::sync::RwLock) docs for details.
///
/// ** unless some type uses interior mutability.  We have made
/// efforts to eradicate interior mutability in this crate.
///
/// Usage conventions:
///
/// ```text
///
/// // property naming:
/// struct Foo {
///     global_state_lock: GlobalStateLock
/// }
///
/// // read guard naming:
/// let global_state = foo.global_state_lock.lock_guard().await;
///
/// // write guard naming:
/// let global_state_mut = foo.global_state_lock.lock_guard_mut().await;
/// ```
///
/// These conventions make it easy to distinguish read access from write
/// access when reading and reviewing code.
///
/// When using a read-guard or write-guard, always drop it as soon as possible.
/// Failure to do so can result in poor concurrency or deadlock.
///
/// Deadlocks are generally not hard to track down.  Lock events are traced.
/// The app log records each `TryAcquire`, `Acquire` and `Release` event
/// when run with `RUST_LOG='info,neptune_core=trace'`.
///
/// If a deadlock has occurred, the log will end with a `TryAcquire` event
/// (read or write) and just scroll up to find the previous `Acquire` for
/// write event to see which thread is holding the lock.
#[derive(Debug, Clone)]
pub struct GlobalStateLock {
    global_state_lock: sync_tokio::AtomicRw<GlobalState>,

    /// The `cli_args::Args` are read-only and accessible by all tasks/threads.
    cli: cli_args::Args,
}

impl GlobalStateLock {
    pub fn new(
        wallet_state: WalletState,
        chain: BlockchainState,
        net: NetworkingState,
        cli: cli_args::Args,
        mempool: Mempool,
        mining: bool,
    ) -> Self {
        let global_state = GlobalState::new(wallet_state, chain, net, cli.clone(), mempool, mining);
        let global_state_lock = sync_tokio::AtomicRw::from((
            global_state,
            Some("GlobalState"),
            Some(crate::LOG_TOKIO_LOCK_EVENT_CB),
        ));

        Self {
            global_state_lock,
            cli,
        }
    }

    // check if mining
    pub async fn mining(&self) -> bool {
        self.lock(|s| s.mining).await
    }

    // enable or disable mining
    pub async fn set_mining(&self, mining: bool) {
        self.lock_mut(|s| s.mining = mining).await
    }

    // flush databases (persist to disk)
    pub async fn flush_databases(&self) -> Result<()> {
        self.lock_guard_mut().await.flush_databases().await
    }

    /// store a coinbase (self-mined) block
    pub async fn store_coinbase_block(
        &self,
        new_block: Block,
        coinbase_utxo_info: ExpectedUtxo,
    ) -> Result<()> {
        self.lock_guard_mut()
            .await
            .set_new_self_mined_tip(new_block, coinbase_utxo_info)
            .await
    }

    /// store a block (non coinbase)
    pub async fn store_block(&self, new_block: Block) -> Result<()> {
        self.lock_guard_mut().await.set_new_tip(new_block).await
    }

    /// resync membership proofs
    pub async fn resync_membership_proofs(&self) -> Result<()> {
        self.lock_guard_mut().await.resync_membership_proofs().await
    }

    pub async fn prune_abandoned_monitored_utxos(
        &self,
        block_depth_threshhold: usize,
    ) -> Result<usize> {
        self.lock_guard_mut()
            .await
            .prune_abandoned_monitored_utxos(block_depth_threshhold)
            .await
    }

    #[inline]
    pub fn cli(&self) -> &cli_args::Args {
        &self.cli
    }

    // Only for tests to simulate different CLI params.
    #[cfg(test)]
    pub async fn set_cli(&mut self, cli: cli_args::Args) {
        self.lock_guard_mut().await.cli = cli.clone();
        self.cli = cli;
    }
}

impl Deref for GlobalStateLock {
    type Target = sync_tokio::AtomicRw<GlobalState>;

    fn deref(&self) -> &Self::Target {
        &self.global_state_lock
    }
}

impl DerefMut for GlobalStateLock {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.global_state_lock
    }
}

/// `GlobalState` handles all state of a Neptune node that is shared across its tasks.
///
/// Some fields are only written to by certain tasks.
#[derive(Debug)]
pub struct GlobalState {
    /// The `WalletState` may be updated by the main task and the RPC server.
    pub wallet_state: WalletState,

    /// The `BlockchainState` may only be updated by the main task.
    pub chain: BlockchainState,

    /// The `NetworkingState` may be updated by both the main task and peer tasks.
    pub net: NetworkingState,

    /// The `cli_args::Args` are read-only and accessible by all tasks.
    cli: cli_args::Args,

    /// The `Mempool` may only be updated by the main task.
    pub mempool: Mempool,

    // Only the mining task should write to this, anyone can read.
    pub mining: bool,
}

impl GlobalState {
    pub fn new(
        wallet_state: WalletState,
        chain: BlockchainState,
        net: NetworkingState,
        cli: cli_args::Args,
        mempool: Mempool,
        mining: bool,
    ) -> Self {
        Self {
            wallet_state,
            chain,
            net,
            cli,
            mempool,
            mining,
        }
    }

    pub async fn get_wallet_status_for_tip(&self) -> WalletStatus {
        let tip_digest = self.chain.light_state().hash();
        self.wallet_state
            .get_wallet_status_from_lock(tip_digest)
            .await
    }

    pub async fn get_latest_balance_height(&self) -> Option<BlockHeight> {
        let (height, time_secs) =
            time_fn_call_async(self.get_latest_balance_height_internal()).await;

        debug!("call to get_latest_balance_height() took {time_secs} seconds");

        height
    }

    /// Retrieve block height of last change to wallet balance.
    ///
    /// note: this fn could be implemented as:
    ///   1. get_balance_history()
    ///   2. sort by height
    ///   3. return height of last entry.
    ///
    /// this implementation is a bit more efficient as it avoids
    ///   the sort and some minor work looking up amount and confirmation
    ///   height of each utxo.
    ///
    /// Presently this is o(n) with the number of monitored utxos.
    /// if storage could keep track of latest spend utxo for the active
    /// tip, then this could be o(1).
    async fn get_latest_balance_height_internal(&self) -> Option<BlockHeight> {
        let current_tip_digest = self.chain.light_state().hash();
        let monitored_utxos = self.wallet_state.wallet_db.monitored_utxos();

        if monitored_utxos.is_empty().await {
            return None;
        }

        let mut max_spent_in_block: Option<BlockHeight> = None;
        let mut max_confirmed_in_block: Option<BlockHeight> = None;

        // monitored_utxos are ordered by confirmed_in_block ascending.
        // To efficiently find max(confirmed_in_block) we can start at the end
        // and work backward until we find the first utxo with a valid
        // membership proof for current tip.
        //
        // We then continue working backward through all entries to
        // determine max(spent_in_block)

        // note: Stream trait does not have a way to reverse, so instead
        // of stream_values() we use stream_many_values() and supply
        // an iterator of indexes that are already reversed.

        let stream = monitored_utxos
            .stream_many_values((0..monitored_utxos.len().await).rev())
            .await;
        pin_mut!(stream); // needed for iteration

        while let Some(mutxo) = stream.next().await {
            if max_confirmed_in_block.is_none() {
                if let Some((.., confirmed_in_block)) = mutxo.confirmed_in_block {
                    if mutxo
                        .get_membership_proof_for_block(current_tip_digest)
                        .is_some()
                    {
                        max_confirmed_in_block = Some(confirmed_in_block);
                    }
                }
            }

            if let Some((.., spent_in_block)) = mutxo.spent_in_block {
                if mutxo
                    .get_membership_proof_for_block(current_tip_digest)
                    .is_some()
                    && (max_spent_in_block.is_none()
                        || max_spent_in_block.is_some_and(|x| x < spent_in_block))
                {
                    max_spent_in_block = Some(spent_in_block);
                }
            }
        }

        max(max_confirmed_in_block, max_spent_in_block)
    }

    /// Retrieve wallet balance history
    pub async fn get_balance_history(&self) -> Vec<(Digest, Timestamp, BlockHeight, NeptuneCoins)> {
        let current_tip_digest = self.chain.light_state().hash();

        let monitored_utxos = self.wallet_state.wallet_db.monitored_utxos();

        // let num_monitored_utxos = monitored_utxos.len();
        let mut history = vec![];

        let stream = monitored_utxos.stream_values().await;
        pin_mut!(stream); // needed for iteration
        while let Some(monitored_utxo) = stream.next().await {
            if monitored_utxo
                .get_membership_proof_for_block(current_tip_digest)
                .is_none()
            {
                continue;
            }

            if let Some((confirming_block, confirmation_timestamp, confirmation_height)) =
                monitored_utxo.confirmed_in_block
            {
                let amount = monitored_utxo.utxo.get_native_currency_amount();
                history.push((
                    confirming_block,
                    confirmation_timestamp,
                    confirmation_height,
                    amount,
                ));
                if let Some((spending_block, spending_timestamp, spending_height)) =
                    monitored_utxo.spent_in_block
                {
                    history.push((spending_block, spending_timestamp, spending_height, -amount));
                }
            }
        }
        history
    }

    /// Generate a primitive witness for a transaction from various disparate witness data.
    pub fn generate_primitive_witness(
        tx_inputs: &TxInputList,
        tx_outputs: &TxOutputList,
        transaction_kernel: TransactionKernel,
        mutator_set_accumulator: MutatorSetAccumulator,
    ) -> PrimitiveWitness {
        let type_scripts = [NativeCurrency.program(), TimeLock.program()]
            .map(TypeScript::new)
            .to_vec();

        let lock_script_witnesses = tx_inputs
            .spending_keys_iter()
            .into_iter()
            .map(|k| k.unlock_key().values().to_vec())
            .collect_vec();

        PrimitiveWitness {
            input_utxos: SaltedUtxos::new(tx_inputs.utxos()),
            input_lock_scripts: tx_inputs.lock_scripts(),
            type_scripts,
            lock_script_witnesses,
            input_membership_proofs: tx_inputs.ms_membership_proofs(),
            output_utxos: SaltedUtxos::new(tx_outputs.utxos()),
            mutator_set_accumulator,
            kernel: transaction_kernel,
        }
    }

    /// generates `TxOutputList` from a list of address:amount pairs (outputs).
    ///
    /// This is a helper method for generating the `TxOutputList` that
    /// is required by create_transaction() and create_raw_transaction().
    ///
    /// For each output, if a wallet key exists for the address then OffChain
    /// notification will be used via `ExpectedUtxo`.  Otherwise OnChainPubKey
    /// notification will be used via `PublicAnnouncement`.
    ///
    /// If a different behavior is desired, the TxOutputList can be
    /// constructed manually.
    pub fn generate_tx_outputs(
        &self,
        outputs: impl IntoIterator<Item = (ReceivingAddressType, NeptuneCoins)>,
    ) -> Result<TxOutputList> {
        let mut tx_outputs = TxOutputList::default();
        let block_height = self.chain.light_state().header().height;

        // Convert outputs.  [address:amount] --> TxOutputList
        for (address, amount) in outputs.into_iter() {
            let sender_randomness = self
                .wallet_state
                .wallet_secret
                .generate_sender_randomness(block_height, address.privacy_digest());

            // append to tx_outputs
            //
            // The UtxoNotifyType (Onchain or Offchain) is automatically detected
            // based on whether the address belongs to our wallet or not.
            tx_outputs.push(TxOutput::auto(
                &self.wallet_state,
                &address,
                amount,
                sender_randomness,
            )?);
        }

        Ok(tx_outputs)
    }

    /// creates a Transaction.
    ///
    /// This API provides a simple-to-use interface for creating a transaction.
    /// Utxo inputs are automatically chosen and a change output is
    /// automatically created, such that:
    ///
    ///   change = sum(inputs) - sum(outputs) - fee.
    ///
    /// When finer control is required, [Self::create_raw_transaction()]
    /// can be used instead.
    ///
    /// The `tx_outputs` parameter should normally be generated with
    /// [Self::generate_tx_outputs].  This will generate OffChain
    /// notifications for UTXOs destined for our wallet and OnChainPubKey
    /// notifications for all other UTXOs.
    ///
    /// It is the caller's responsibility to inform the wallet of any expected
    /// utxos, ie offchain secret notifications, for utxos that match wallet
    /// keys.
    ///
    /// This function will modify the `tx_outputs` parameter by
    /// appending an element representing the change output, if change is
    /// needed.  Expected utxos, including change can then be retrieved
    /// with [TxOutputList::expected_utxos()].
    ///
    /// The `change_utxo_notify_method` parameter should normally be
    /// UtxoNotifyMethod::OffChain in order to save blockchain space.
    /// Note however there is a risk of losing funds with offchain
    /// notification if local state is lost.
    ///
    /// Note that `create_transaction()` does not modify any state and does not
    /// require acquiring write lock.  This is important becauce internally it
    /// calls prove() which is a very lengthy operation.
    ///
    /// Example:
    ///
    /// ```compile_fail
    ///
    /// // we obtain a change_address first, as it requires modifying wallet state.
    /// let change_spending_key = global_state_lock
    ///     .lock_guard_mut()
    ///     .await
    ///     .wallet_state
    ///     .wallet_secret
    ///     .next_unused_generation_spending_key();
    ///
    /// // obtain read lock
    /// let state = self.state.lock_guard().await;
    /// let mut tx_outputs = state.generate_tx_outputs(outputs)?;
    ///
    /// // Create the transaction
    /// let transaction = state
    ///     .create_transaction(
    ///         &mut tx_outputs,
    ///         change_spending_key.into(),
    ///         UtxoNotifyMethod::OffChain,   // notify change utxo offchain
    ///         NeptuneCoins::new(2),         // fee
    ///         Timestamp::now(),
    ///     )
    ///     .await?;
    /// drop(state);
    ///
    /// // Inform wallet of any expected incoming utxos.
    /// state
    ///     .lock_guard_mut()
    ///     .await
    ///     .add_expected_utxos_to_wallet(tx_outputs.expected_utxos())
    ///     .await?;
    /// ```
    pub async fn create_transaction(
        &self,
        tx_outputs: &mut TxOutputList,
        change_spending_key: SpendingKeyType,
        change_utxo_notify_method: UtxoNotifyMethod,
        fee: NeptuneCoins,
        timestamp: Timestamp,
    ) -> Result<Transaction> {
        // 1. create/add change output if necessary.
        let total_spend = tx_outputs.total_native_coins() + fee;

        let tip_hash = self.chain.light_state().hash();

        // collect spendable inputs
        let tx_inputs = self
            .wallet_state
            .allocate_sufficient_input_funds_from_lock(total_spend, tip_hash, timestamp)
            .await?;

        let input_amount = tx_inputs.total_native_coins();

        if total_spend < input_amount {
            let block_height = self.chain.light_state().header().height;
            let change_address = change_spending_key.to_address();

            let amount = input_amount.checked_sub(&total_spend).ok_or_else(|| {
                anyhow::anyhow!("underflow subtracting total_spend from input_amount")
            })?;

            let utxo = Utxo::new_native_coin(change_address.lock_script(), amount);

            let sender_randomness = self
                .wallet_state
                .wallet_secret
                .generate_sender_randomness(block_height, change_address.privacy_digest());

            let tx_output = match change_utxo_notify_method {
                UtxoNotifyMethod::OnChainPubKey => {
                    let public_announcement =
                        change_address.generate_public_announcement(&utxo, sender_randomness)?;
                    TxOutput::onchain_pubkey(
                        utxo,
                        sender_randomness,
                        change_address.privacy_digest(),
                        public_announcement,
                    )
                }
                UtxoNotifyMethod::OnChainSymmetricKey => unimplemented!(),
                UtxoNotifyMethod::OffChain => TxOutput::offchain(
                    utxo,
                    sender_randomness,
                    change_spending_key.privacy_preimage(),
                ),
            };

            tx_outputs.push(tx_output);
        }

        // 2. Create the transaction
        let transaction = self
            .create_raw_transaction(tx_inputs, tx_outputs.clone(), fee, timestamp)
            .await?;

        Ok(transaction)
    }

    /// creates a Transaction.
    ///
    /// This API provides the caller complete control over selection of inputs
    /// and outputs.  When fine grained control is not required,
    /// [Self::create_transaction()] is easier to use and should be preferred.
    ///
    /// It is the caller's responsibility to provide inputs and outputs such
    /// that sum(inputs) == sum(outputs) + fee.  Else an error will result.
    ///
    /// Note that this means the caller must calculate the change amount if any
    /// and provide an output for the change.
    ///
    /// The `tx_outputs` parameter should normally be generated with
    /// [Self::generate_tx_outputs()].  This will generate OffChain
    /// notifications for UTXOs destined for our wallet and OnChainPubKey
    /// notifications for all other UTXOs.
    ///
    /// It is the caller's responsibility to inform the wallet of any expected
    /// utxos, ie offchain secret notifications, for utxos that match wallet
    /// keys.
    ///
    /// Note that `create_raw_transaction()` does not modify any state and does
    /// not require acquiring write lock.  This is important becauce internally
    /// it calls prove() which is a very lengthy operation.
    ///
    /// Example:
    ///
    /// See the implementation of [Self::create_transaction()].
    pub async fn create_raw_transaction(
        &self,
        tx_inputs: TxInputList,
        tx_outputs: TxOutputList,
        fee: NeptuneCoins,
        timestamp: Timestamp,
    ) -> Result<Transaction> {
        // UTXO data: inputs, outputs, and supporting witness data
        let tx_data = self
            .generate_tx_details_for_transaction(tx_inputs, tx_outputs, fee, timestamp)
            .await?;

        self.create_transaction_from_data(tx_data).await
    }

    /// This is a simple wrapper around create_transaction
    /// for compatibility with existing tests.
    #[cfg(test)]
    pub async fn create_transaction_test_wrapper(
        &self,
        tx_output_vec: Vec<TxOutput>,
        fee: NeptuneCoins,
        timestamp: Timestamp,
    ) -> Result<(Transaction, Vec<ExpectedUtxo>)> {
        let mut tx_outputs = TxOutputList::from(tx_output_vec);

        // note: should use next_unused_generation_spending_key()
        // but that requires &mut self.
        let change_spending_key = self
            .wallet_state
            .wallet_secret
            .nth_generation_spending_key(0);

        let len = tx_outputs.len();
        let transaction = self
            .create_transaction(
                &mut tx_outputs,
                change_spending_key.into(),
                UtxoNotifyMethod::OffChain,
                fee,
                timestamp,
            )
            .await?;
        info!("receivers len before: {len}, after: {}", tx_outputs.len());
        Ok((transaction, (&tx_outputs).into()))
    }

    /// Given a list of UTXOs with receiver data, assemble owned and synced and spendable
    /// UTXOs that unlock enough funds, add (and track) a change UTXO if necessary, and
    /// and produce a list of removal records, input UTXOs (with lock scripts and
    /// membership proofs), addition records, and output UTXOs.
    async fn generate_tx_details_for_transaction(
        &self,
        tx_inputs: TxInputList,
        tx_outputs: TxOutputList,
        fee: NeptuneCoins,
        timestamp: Timestamp,
    ) -> Result<TransactionDetails> {
        // total amount to be spent -- determines how many and which UTXOs to use
        let total_spend: NeptuneCoins = tx_outputs.total_native_coins() + fee;
        let input_amount = tx_inputs.total_native_coins();

        // sanity check: do we even have enough funds?
        if total_spend > input_amount {
            debug!("Insufficient funds. total_spend: {total_spend}, input_amount: {input_amount}");
            bail!("Not enough available funds.");
        }
        if total_spend < input_amount {
            let diff = total_spend - input_amount;
            bail!("Missing change output in the amount of {}", diff);
        }

        Ok(TransactionDetails {
            tx_inputs,
            tx_outputs,
            fee,
            timestamp,
        })
    }

    /// Assembles a transaction kernel and supporting witness or proof(s) from
    /// the given transaction data.
    async fn create_transaction_from_data(
        &self,
        transaction_details: TransactionDetails,
    ) -> Result<Transaction> {
        let mutator_set_accumulator = self
            .chain
            .light_state()
            .kernel
            .body
            .mutator_set_accumulator
            .clone();
        let privacy = self.cli().privacy;

        // note: this executes the prover which can take a very
        //       long time, perhaps minutes.  As such, we use
        //       spawn_blocking() to execute on tokio's blocking
        //       threadpool and avoid blocking the tokio executor
        //       and other async tasks.
        let transaction = tokio::task::spawn_blocking(move || {
            Self::create_transaction_from_data_worker(
                transaction_details,
                mutator_set_accumulator,
                privacy,
            )
        })
        .await?;
        Ok(transaction)
    }

    // note: this executes the prover which can take a very
    //       long time, perhaps minutes. It should never be
    //       called directly.
    //       Use create_transaction_from_data() instead.
    //
    // fixme: why is _privacy param unused?
    fn create_transaction_from_data_worker(
        transaction_details: TransactionDetails,
        mutator_set_accumulator: MutatorSetAccumulator,
        _privacy: bool,
    ) -> Transaction {
        let TransactionDetails {
            tx_inputs,
            tx_outputs,
            fee,
            timestamp,
        } = transaction_details;

        // complete transaction kernel
        let kernel = TransactionKernel {
            inputs: tx_inputs.removal_records(&mutator_set_accumulator),
            outputs: tx_outputs.addition_records(),
            public_announcements: tx_outputs.public_announcements(),
            fee,
            timestamp,
            coinbase: None,
            mutator_set_hash: mutator_set_accumulator.hash(),
        };

        // populate witness
        let primitive_witness = Self::generate_primitive_witness(
            &tx_inputs,
            &tx_outputs,
            kernel.clone(),
            mutator_set_accumulator,
        );

        // Convert the validity tree into a single proof.
        // Down the line we want to support proving only the lock scripts, or only
        // the lock scripts and removal records integrity, but nothing else.
        // That's a concern for later though.
        let mut transaction_validity_logic = TransactionValidationLogic::from(primitive_witness);
        transaction_validity_logic.vast.prove();
        transaction_validity_logic.maybe_primitive_witness = None;
        Transaction {
            kernel,
            witness: transaction_validity_logic,
        }
    }

    // If any output UTXO(s) are going back to our wallet (eg change utxo)
    // we add them to pool of expected incoming UTXOs so that we can
    // synchronize them after the Tx is confirmed.
    //
    // Discussion: https://github.com/Neptune-Crypto/neptune-core/pull/136
    pub async fn add_expected_utxos_to_wallet(
        &mut self,
        expected_utxos: impl IntoIterator<Item = ExpectedUtxo>,
    ) -> Result<()> {
        for expected_utxo in expected_utxos.into_iter() {
            self.wallet_state.expected_utxos.add_expected_utxo(
                expected_utxo.utxo,
                expected_utxo.sender_randomness,
                expected_utxo.receiver_preimage,
                expected_utxo.received_from,
            )?;
        }
        Ok(())
    }

    pub async fn get_own_handshakedata(&self) -> HandshakeData {
        HandshakeData {
            tip_header: self.chain.light_state().header().clone(),
            // TODO: Should be `None` if incoming connections are not accepted
            listen_port: Some(self.cli().peer_port),
            network: self.cli().network,
            instance_id: self.net.instance_id,
            version: VERSION.to_string(),
            // For now, all nodes are archival nodes
            is_archival_node: self.chain.is_archival_node(),
        }
    }

    /// In case the wallet database is corrupted or deleted, this method will restore
    /// monitored UTXO data structures from recovery data. This method should only be
    /// called on startup, not while the program is running, since it will only restore
    /// a wallet state, if the monitored UTXOs have been deleted. Not merely if they
    /// are not synced with a valid mutator set membership proof. And this corruption
    /// can only happen if the wallet database is deleted or corrupted.
    pub(crate) async fn restore_monitored_utxos_from_recovery_data(&mut self) -> Result<()> {
        let tip_hash = self.chain.light_state().hash();
        let ams_ref = &self.chain.archival_state().archival_mutator_set;

        assert_eq!(
            tip_hash,
            ams_ref.get_sync_label().await,
            "Archival mutator set must be synced to tip for successful MUTXO recovery"
        );

        // Fetch all incoming UTXOs from recovery data
        let incoming_utxos = self.wallet_state.read_utxo_ms_recovery_data().await?;
        let incoming_utxo_count = incoming_utxos.len();
        info!("Checking {} incoming UTXOs", incoming_utxo_count);

        // Loop over all `incoming_utxos` and check if they have a corresponding
        // monitored UTXO in the database. All monitored UTXOs are fetched outside
        // of the loop to avoid DB access/IO inside the loop.
        let mut recovery_data_for_missing_mutxos = vec![];

        {
            let stream = self
                .wallet_state
                .wallet_db
                .monitored_utxos()
                .stream_values()
                .await;
            pin_mut!(stream); // needed for iteration

            '_outer: for incoming_utxo in incoming_utxos.into_iter() {
                'inner: while let Some(monitored_utxo) = stream.next().await {
                    if monitored_utxo.utxo == incoming_utxo.utxo {
                        let msmp_res = monitored_utxo.get_latest_membership_proof_entry();
                        let msmp = match msmp_res {
                            Some((_blockh_hash, msmp_val)) => msmp_val,
                            None => continue 'inner,
                        };

                        // If UTXO matches, then check if the AOCL index is also a match.
                        // If it is, then the UTXO is already in the wallet database.
                        if msmp.auth_path_aocl.leaf_index == incoming_utxo.aocl_index {
                            continue '_outer;
                        }
                    }
                }

                // If no match is found, add the UTXO to the list of missing UTXOs
                recovery_data_for_missing_mutxos.push(incoming_utxo);
            }
        }

        if recovery_data_for_missing_mutxos.is_empty() {
            info!(
                "No missing monitored UTXOs found in wallet database. Wallet database looks good."
            );
            return Ok(());
        }

        // For all recovery data where we did not find a matching monitored UTXO,
        // recover the MS membership proof, and insert a new monitored UTXO into the
        // wallet database.
        info!(
            "Attempting to restore {} missing monitored UTXOs to wallet database",
            recovery_data_for_missing_mutxos.len()
        );
        let current_aocl_leaf_count = ams_ref.ams().aocl.count_leaves().await;
        let mut restored_mutxos = 0;
        for incoming_utxo in recovery_data_for_missing_mutxos {
            // If the referenced UTXO is in the future from our tip, do not attempt to recover it. Instead: warn the user of this.
            if current_aocl_leaf_count <= incoming_utxo.aocl_index {
                warn!("Cannot restore UTXO with AOCL index {} because it is in the future from our tip. Current AOCL leaf count is {current_aocl_leaf_count}. Maybe this UTXO can be recovered once more blocks are downloaded from peers?", incoming_utxo.aocl_index);
                continue;
            }
            let ms_item = Hash::hash(&incoming_utxo.utxo);
            let restored_msmp_res = ams_ref
                .ams()
                .restore_membership_proof(
                    ms_item,
                    incoming_utxo.sender_randomness,
                    incoming_utxo.receiver_preimage,
                    incoming_utxo.aocl_index,
                )
                .await;
            let restored_msmp = match restored_msmp_res {
                Ok(msmp) => {
                    // Verify that the restored MSMP is valid
                    if !ams_ref.ams().verify(ms_item, &msmp).await {
                        warn!("Restored MSMP is invalid. Skipping restoration of UTXO with AOCL index {}. Maybe this UTXO is on an abandoned chain?", incoming_utxo.aocl_index);
                        continue;
                    }

                    msmp
                }
                Err(err) => bail!("Could not restore MS membership proof. Got: {err}"),
            };

            let mut restored_mutxo =
                MonitoredUtxo::new(incoming_utxo.utxo, self.wallet_state.number_of_mps_per_utxo);
            restored_mutxo.add_membership_proof_for_tip(tip_hash, restored_msmp);

            self.wallet_state
                .wallet_db
                .monitored_utxos_mut()
                .push(restored_mutxo)
                .await;
            restored_mutxos += 1;
        }

        self.wallet_state.wallet_db.persist().await;
        info!("Successfully restored {restored_mutxos} monitored UTXOs to wallet database");

        Ok(())
    }

    ///  Locking:
    ///   * acquires `monitored_utxos_lock` for write
    pub async fn resync_membership_proofs_from_stored_blocks(
        &mut self,
        tip_hash: Digest,
    ) -> Result<()> {
        // loop over all monitored utxos
        let monitored_utxos = self.wallet_state.wallet_db.monitored_utxos_mut();

        'outer: for i in 0..monitored_utxos.len().await {
            let i = i as Index;
            let monitored_utxo = monitored_utxos.get(i).await;

            // Ignore those MUTXOs that were marked as abandoned
            if monitored_utxo.abandoned_at.is_some() {
                continue;
            }

            // ignore synced ones
            if monitored_utxo.is_synced_to(tip_hash) {
                continue;
            }

            debug!(
                "Resyncing monitored UTXO number {}, with hash {}",
                i,
                Hash::hash(&monitored_utxo.utxo)
            );

            // If the UTXO was not confirmed yet, there is no
            // point in synchronizing its membership proof.
            let (confirming_block_digest, confirming_block_height) =
                match monitored_utxo.confirmed_in_block {
                    Some((confirmed_block_hash, _timestamp, block_height)) => {
                        (confirmed_block_hash, block_height)
                    }
                    None => {
                        continue;
                    }
                };

            // try latest (block hash, membership proof) entry
            let (block_hash, mut membership_proof) = monitored_utxo
                .get_latest_membership_proof_entry()
                .expect("Database not in consistent state. Monitored UTXO must have at least one membership proof.");

            // request path-to-tip
            let (backwards, _luca, forwards) = self
                .chain
                .archival_state()
                .find_path(block_hash, tip_hash)
                .await;

            // after this point, we may be modifying it.
            let mut monitored_utxo = monitored_utxo.clone();

            // walk backwards, reverting
            for revert_block_hash in backwards.into_iter() {
                // Was the UTXO confirmed in this block? If so, there
                // is nothing we can do except orphan the UTXO: that
                // is, leave it without a synced membership proof.
                // Whenever current owned UTXOs are queried, one
                // should take care to filter for UTXOs that have a
                // membership proof synced to the current block tip.
                if confirming_block_digest == revert_block_hash {
                    warn!(
                        "Could not recover MSMP as transaction appears to be on an abandoned chain"
                    );
                    break 'outer;
                }

                let revert_block = self
                    .chain
                    .archival_state()
                    .get_block(revert_block_hash)
                    .await?
                    .unwrap();
                let maybe_revert_block_predecessor = self
                    .chain
                    .archival_state()
                    .get_block(revert_block.kernel.header.prev_block_digest)
                    .await?;
                let previous_mutator_set = match maybe_revert_block_predecessor {
                    Some(block) => block.kernel.body.mutator_set_accumulator.clone(),
                    None => MutatorSetAccumulator::default(),
                };

                debug!("MUTXO confirmed at height {confirming_block_height}, reverting for height {} on abandoned chain", revert_block.kernel.header.height);

                // revert removals
                let removal_records = revert_block.kernel.body.transaction.kernel.inputs.clone();
                for removal_record in removal_records.iter().rev() {
                    // membership_proof.revert_update_from_removal(&removal);
                    membership_proof
                        .revert_update_from_remove(removal_record)
                        .expect("Could not revert membership proof from removal record.");
                }

                // revert additions
                membership_proof.revert_update_from_batch_addition(&previous_mutator_set);

                // unset spent_in_block field if the UTXO was spent in this block
                if let Some((spent_block_hash, _, _)) = monitored_utxo.spent_in_block {
                    if spent_block_hash == revert_block_hash {
                        monitored_utxo.spent_in_block = None;
                    }
                }

                // assert valid (if unspent)
                assert!(monitored_utxo.spent_in_block.is_some() || previous_mutator_set
                    .verify(Hash::hash(&monitored_utxo.utxo), &membership_proof), "Failed to verify monitored UTXO {monitored_utxo:?}\n against previous MSA in block {revert_block:?}");
            }

            // walk forwards, applying
            for apply_block_hash in forwards.into_iter() {
                // Was the UTXO confirmed in this block?
                // This can occur in some edge cases of forward-only
                // resynchronization. In this case, assume the
                // membership proof is already synced to this block.
                if confirming_block_digest == apply_block_hash {
                    continue;
                }

                let apply_block = self
                    .chain
                    .archival_state()
                    .get_block(apply_block_hash)
                    .await?
                    .unwrap();
                let maybe_apply_block_predecessor = self
                    .chain
                    .archival_state()
                    .get_block(apply_block.kernel.header.prev_block_digest)
                    .await?;
                let mut block_msa = match maybe_apply_block_predecessor {
                    Some(block) => block.kernel.body.mutator_set_accumulator.clone(),
                    None => MutatorSetAccumulator::default(),
                };
                let addition_records = apply_block.kernel.body.transaction.kernel.outputs.clone();
                let removal_records = apply_block.kernel.body.transaction.kernel.inputs.clone();

                // apply additions
                for addition_record in addition_records.iter() {
                    membership_proof
                        .update_from_addition(
                            Hash::hash(&monitored_utxo.utxo),
                            &block_msa,
                            addition_record,
                        )
                        .expect("Could not update membership proof with addition record.");
                    block_msa.add(addition_record);
                }

                // apply removals
                for removal_record in removal_records.iter() {
                    membership_proof
                        .update_from_remove(removal_record)
                        .expect("Could not update membership proof from removal record.");
                    block_msa.remove(removal_record);
                }

                assert_eq!(block_msa, apply_block.kernel.body.mutator_set_accumulator);
            }

            // store updated membership proof
            monitored_utxo.add_membership_proof_for_tip(tip_hash, membership_proof);

            // update storage.
            monitored_utxos.set(i, monitored_utxo).await
        }

        // Update sync label and persist
        self.wallet_state.wallet_db.set_sync_label(tip_hash).await;
        self.wallet_state.wallet_db.persist().await;

        Ok(())
    }

    /// Delete from the database all monitored UTXOs from abandoned chains with a depth deeper than
    /// `block_depth_threshhold`. Use `prune_mutxos_of_unknown_depth = true` to remove MUTXOs from
    /// abandoned chains of unknown depth.
    /// Returns the number of monitored UTXOs removed from the database.
    ///
    /// Locking:
    ///  * acquires `monitored_utxos` lock for write
    pub async fn prune_abandoned_monitored_utxos<'a>(
        &mut self,
        block_depth_threshhold: usize,
    ) -> Result<usize> {
        const MIN_BLOCK_DEPTH_FOR_MUTXO_PRUNING: usize = 10;
        if block_depth_threshhold < MIN_BLOCK_DEPTH_FOR_MUTXO_PRUNING {
            bail!(
                "
                Cannot prune monitored UTXOs with a depth threshold less than
                {MIN_BLOCK_DEPTH_FOR_MUTXO_PRUNING}. Got threshold {block_depth_threshhold}"
            )
        }

        let current_tip_header = self.chain.light_state().header();
        let current_tip_digest = self.chain.light_state().hash();

        let current_tip_info: (Digest, Timestamp, BlockHeight) = (
            current_tip_digest,
            current_tip_header.timestamp,
            current_tip_header.height,
        );

        let monitored_utxos = self.wallet_state.wallet_db.monitored_utxos_mut();
        let mut removed_count = 0;

        // Find monitored_utxo for updating
        for i in 0..monitored_utxos.len().await {
            let mut mutxo = monitored_utxos.get(i).await;

            // 1. Spent MUTXOs are not marked as abandoned, as there's no reason to maintain them
            //    once the spending block is buried sufficiently deep
            // 2. If synced to current tip, there is nothing more to do with this MUTXO
            // 3. If already marked as abandoned, we don't do that again
            if mutxo.spent_in_block.is_some()
                || mutxo.is_synced_to(current_tip_info.0)
                || mutxo.abandoned_at.is_some()
            {
                continue;
            }

            // MUTXO is neither spent nor synced. Mark as abandoned
            // if it was confirmed in block that is now abandoned,
            // and if that block is older than threshold.
            if let Some((_, _, block_height_confirmed)) = mutxo.confirmed_in_block {
                let depth = current_tip_header.height - block_height_confirmed + 1;

                let abandoned = depth >= block_depth_threshhold as i128
                    && mutxo
                        .was_abandoned(current_tip_digest, self.chain.archival_state())
                        .await;

                if abandoned {
                    mutxo.abandoned_at = Some(current_tip_info);
                    monitored_utxos.set(i, mutxo).await;
                    removed_count += 1;
                }
            }
        }

        Ok(removed_count)
    }

    pub async fn flush_databases(&mut self) -> Result<()> {
        // flush wallet databases
        self.wallet_state.wallet_db.persist().await;

        // flush block_index database
        self.chain.archival_state_mut().block_index_db.flush().await;

        // persist archival_mutator_set, with sync label
        let hash = self.chain.archival_state().get_tip().await.hash();
        self.chain
            .archival_state_mut()
            .archival_mutator_set
            .set_sync_label(hash)
            .await;

        self.chain
            .archival_state_mut()
            .archival_mutator_set
            .persist()
            .await;

        // flush peer_standings
        self.net.peer_databases.peer_standings.flush().await;

        debug!("Flushed all databases");

        Ok(())
    }

    /// Update client's state with a new block. Block is assumed to be valid, also wrt. to PoW.
    /// The received block will be set as the new tip, regardless of its accumulated PoW.
    pub async fn set_new_tip(&mut self, new_block: Block) -> Result<()> {
        self.set_new_tip_internal(new_block, None).await
    }

    /// Update client's state with a new block that was mined locally. Block is assumed to be valid,
    /// also wrt. to PoW. The received block will be set as the new tip, regardless of its
    /// accumulated PoW.
    pub async fn set_new_self_mined_tip(
        &mut self,
        new_block: Block,
        coinbase_utxo_info: ExpectedUtxo,
    ) -> Result<()> {
        self.set_new_tip_internal(new_block, Some(coinbase_utxo_info))
            .await
    }

    /// Update client's state with a new block. Block is assumed to be valid, also wrt. to PoW.
    /// The received block will be set as the new tip, regardless of its accumulated PoW. or its
    /// validity.
    async fn set_new_tip_internal(
        &mut self,
        new_block: Block,
        coinbase_utxo_info: Option<ExpectedUtxo>,
    ) -> Result<()> {
        // note: we make this fn internal so we can log its duration and ensure it will
        // never be called directly by another fn, without the timings.
        async fn set_new_tip_internal_worker(
            myself: &mut GlobalState,
            new_block: Block,
            coinbase_utxo_info: Option<ExpectedUtxo>,
        ) -> Result<()> {
            // Apply the updates
            myself
                .chain
                .archival_state_mut()
                .write_block_as_tip(&new_block)
                .await?;

            // update the mutator set with the UTXOs from this block
            myself
                .chain
                .archival_state_mut()
                .update_mutator_set(&new_block)
                .await
                .expect("Updating mutator set must succeed");

            if let Some(coinbase_info) = coinbase_utxo_info {
                // Notify wallet to expect the coinbase UTXO, as we mined this block
                myself
                    .wallet_state
                    .expected_utxos
                    .add_expected_utxo(
                        coinbase_info.utxo,
                        coinbase_info.sender_randomness,
                        coinbase_info.receiver_preimage,
                        UtxoNotifier::OwnMiner,
                    )
                    .expect("UTXO notification from miner must be accepted");
            }

            // Get parent of tip for mutator-set data needed for various updates. Parent of the
            // stored block will always exist since all blocks except the genesis block have a
            // parent, and the genesis block is considered code, not data, so the genesis block
            // will never be changed or updated through this method.
            let tip_parent = myself
                .chain
                .archival_state()
                .get_tip_parent()
                .await
                .expect("Parent must exist when storing a new block");

            // Sanity check that must always be true for a valid block
            assert_eq!(
                tip_parent.hash(),
                new_block.header().prev_block_digest,
                "Tip parent has must match indicated parent hash"
            );
            let previous_ms_accumulator = tip_parent.body().mutator_set_accumulator.clone();

            // update wallet state with relevant UTXOs from this block
            myself
                .wallet_state
                .update_wallet_state_with_new_block(&previous_ms_accumulator, &new_block)
                .await?;

            // Update mempool with UTXOs from this block. This is done by removing all transaction
            // that became invalid/was mined by this block.
            myself
                .mempool
                .update_with_block(previous_ms_accumulator, &new_block)
                .await;

            myself.chain.light_state_mut().set_block(new_block);

            // Flush databases
            myself.flush_databases().await?;

            Ok(())
        }

        crate::macros::duration_async_info!(set_new_tip_internal_worker(
            self,
            new_block,
            coinbase_utxo_info
        ))
    }

    /// resync membership proofs
    pub async fn resync_membership_proofs(&mut self) -> Result<()> {
        // Do not fix memberhip proofs if node is in sync mode, as we would otherwise
        // have to sync many times, instead of just *one* time once we have caught up.
        if self.net.syncing {
            debug!("Not syncing MS membership proofs because we are syncing");
            return Ok(());
        }

        // is it necessary?
        let current_tip_digest = self.chain.light_state().hash();
        if self.wallet_state.is_synced_to(current_tip_digest).await {
            debug!("Membership proof syncing not needed");
            return Ok(());
        }

        // do we have blocks?
        if self.chain.is_archival_node() {
            return self
                .resync_membership_proofs_from_stored_blocks(current_tip_digest)
                .await;
        }

        // request blocks from peers
        todo!("We don't yet support non-archival nodes");

        // Ok(())
    }

    #[inline]
    pub fn cli(&self) -> &cli_args::Args {
        &self.cli
    }
}

#[cfg(test)]
mod global_state_tests {
    use crate::{
        config_models::network::Network,
        models::{blockchain::block::Block, state::wallet::utxo_notification_pool::UtxoNotifier},
        tests::shared::{
            add_block_to_light_state, make_mock_block, make_mock_block_with_valid_pow,
            mock_genesis_global_state, mock_genesis_wallet_state,
        },
    };
    use num_traits::{One, Zero};
    use rand::{rngs::StdRng, thread_rng, Rng, SeedableRng};
    use tracing_test::traced_test;

    use super::{wallet::WalletSecret, *};

    async fn wallet_state_has_all_valid_mps_for(
        wallet_state: &WalletState,
        tip_block: &Block,
    ) -> bool {
        let monitored_utxos = wallet_state.wallet_db.monitored_utxos();
        for monitored_utxo in monitored_utxos.get_all().await.iter() {
            let current_mp = monitored_utxo.get_membership_proof_for_block(tip_block.hash());

            match current_mp {
                Some(mp) => {
                    if !tip_block
                        .kernel
                        .body
                        .mutator_set_accumulator
                        .verify(Hash::hash(&monitored_utxo.utxo), &mp)
                    {
                        return false;
                    }
                }
                None => return false,
            }
        }

        true
    }

    #[traced_test]
    #[tokio::test]
    async fn premine_recipient_cannot_spend_premine_before_and_can_after_release_date() {
        let network = Network::RegTest;
        let other_wallet = WalletSecret::new_random();
        let global_state_lock =
            mock_genesis_global_state(network, 2, WalletSecret::devnet_wallet()).await;
        let genesis_block = Block::genesis_block(network);
        let twenty_neptune: NeptuneCoins = NeptuneCoins::new(20);
        let twenty_coins = twenty_neptune.to_native_coins();
        let recipient_address = other_wallet.nth_generation_spending_key(0).to_address();
        let main_lock_script = recipient_address.lock_script();
        let output_utxo = Utxo {
            coins: twenty_coins,
            lock_script_hash: main_lock_script.hash(),
        };
        let sender_randomness = Digest::default();
        let receiver_privacy_digest = recipient_address.privacy_digest;
        let public_announcement = recipient_address
            .generate_public_announcement(&output_utxo, sender_randomness)
            .unwrap();
        let tx_outputs = vec![TxOutput::onchain_pubkey(
            output_utxo.clone(),
            sender_randomness,
            receiver_privacy_digest,
            public_announcement,
        )];

        let monitored_utxos = global_state_lock
            .lock_guard()
            .await
            .wallet_state
            .wallet_db
            .monitored_utxos()
            .get_all()
            .await;
        assert_ne!(monitored_utxos.len(), 0);

        // one month before release date, we should not be able to create the transaction
        let launch = genesis_block.kernel.header.timestamp;
        let six_months = Timestamp::months(6);
        let one_month = Timestamp::months(1);
        assert!(global_state_lock
            .lock_guard()
            .await
            .create_transaction_test_wrapper(
                tx_outputs.clone(),
                NeptuneCoins::new(1),
                launch + six_months - one_month,
            )
            .await
            .is_err());

        // one month after though, we should be
        let (mut tx, _) = global_state_lock
            .lock_guard()
            .await
            .create_transaction_test_wrapper(
                tx_outputs,
                NeptuneCoins::new(1),
                launch + six_months + one_month,
            )
            .await
            .unwrap();
        assert!(tx.is_valid());

        // but if we backdate the timestamp two months, not anymore!
        tx.kernel.timestamp = tx.kernel.timestamp - Timestamp::months(2);
        // we can't test this yet; we don't have tasm code for time locks yet!
        // todo: uncomment the next line when we do.
        // assert!(!tx.is_valid());
        tx.kernel.timestamp = tx.kernel.timestamp + Timestamp::months(2);

        assert_eq!(
            2,
            tx.kernel.outputs.len(),
            "tx must have a send output and a change output"
        );
        assert_eq!(
            1,
            tx.kernel.inputs.len(),
            "tx must have exactly one input, a genesis UTXO"
        );

        // Test with a transaction with three outputs and one (premine) input
        let mut other_tx_outputs = vec![];
        let mut output_utxos: Vec<Utxo> = vec![];
        for i in 2..5 {
            let amount: NeptuneCoins = NeptuneCoins::new(i);
            let that_many_coins = amount.to_native_coins();
            let receiving_address = other_wallet.nth_generation_spending_key(0).to_address();
            let lock_script = receiving_address.lock_script();
            let utxo = Utxo {
                coins: that_many_coins,
                lock_script_hash: lock_script.hash(),
            };
            let other_sender_randomness = Digest::default();
            let other_receiver_digest = receiving_address.privacy_digest;
            let other_public_announcement = receiving_address
                .generate_public_announcement(&utxo, other_sender_randomness)
                .unwrap();
            output_utxos.push(utxo.clone());
            other_tx_outputs.push(TxOutput::onchain_pubkey(
                utxo,
                other_sender_randomness,
                other_receiver_digest,
                other_public_announcement,
            ));
        }

        let (new_tx, _) = global_state_lock
            .lock_guard()
            .await
            .create_transaction_test_wrapper(
                other_tx_outputs,
                NeptuneCoins::new(1),
                launch + six_months + one_month,
            )
            .await
            .unwrap();
        assert!(new_tx.is_valid());
        assert_eq!(
            4,
            new_tx.kernel.outputs.len(),
            "tx must have three send outputs and a change output"
        );
        assert_eq!(
            1,
            new_tx.kernel.inputs.len(),
            "tx must have exactly one input, a genesis UTXO"
        );
    }

    #[traced_test]
    #[tokio::test]
    async fn restore_monitored_utxos_from_recovery_data_test() {
        let mut rng = thread_rng();
        let network = Network::RegTest;
        let devnet_wallet = WalletSecret::devnet_wallet();
        let global_state_lock = mock_genesis_global_state(network, 2, devnet_wallet).await;
        let mut global_state = global_state_lock.lock_guard_mut().await;
        let other_receiver_address = WalletSecret::new_random()
            .nth_generation_spending_key(0)
            .to_address();
        let genesis_block = Block::genesis_block(network);
        let (mock_block_1, _, _) =
            make_mock_block(&genesis_block, None, other_receiver_address, rng.gen());
        crate::tests::shared::add_block_to_archival_state(
            global_state.chain.archival_state_mut(),
            mock_block_1.clone(),
        )
        .await
        .unwrap();
        add_block_to_light_state(global_state.chain.light_state_mut(), mock_block_1.clone())
            .await
            .unwrap();

        // Delete everything from monitored UTXO (the premined UTXO)
        {
            let monitored_utxos = global_state.wallet_state.wallet_db.monitored_utxos_mut();
            assert!(
                monitored_utxos.len().await.is_one(),
                "MUTXO must have genesis element before emptying it"
            );
            monitored_utxos.pop().await;

            assert!(
                monitored_utxos.is_empty().await,
                "MUTXO must be empty after emptying it"
            );
        }

        // Recover the MUTXO from the recovery data, and verify that MUTXOs are restored
        global_state
            .restore_monitored_utxos_from_recovery_data()
            .await
            .unwrap();
        {
            let monitored_utxos = global_state.wallet_state.wallet_db.monitored_utxos();
            assert!(
                monitored_utxos.len().await.is_one(),
                "MUTXO must have genesis element after recovering it"
            );

            // Verify that the restored MUTXO has a valid MSMP
            let own_premine_mutxo = monitored_utxos.get(0).await;
            let ms_item = Hash::hash(&own_premine_mutxo.utxo);
            global_state
                .chain
                .light_state()
                .body()
                .mutator_set_accumulator
                .verify(
                    ms_item,
                    &own_premine_mutxo
                        .get_latest_membership_proof_entry()
                        .unwrap()
                        .1,
                );
            assert_eq!(
                mock_block_1.hash(),
                own_premine_mutxo
                    .get_latest_membership_proof_entry()
                    .unwrap()
                    .0,
                "MUTXO must have the correct latest block digest value"
            );
        }
    }

    #[traced_test]
    #[tokio::test]
    async fn resync_ms_membership_proofs_simple_test() -> Result<()> {
        let mut rng = thread_rng();
        let network = Network::RegTest;
        let global_state_lock =
            mock_genesis_global_state(network, 2, WalletSecret::devnet_wallet()).await;
        let mut global_state = global_state_lock.lock_guard_mut().await;

        let other_receiver_wallet_secret = WalletSecret::new_random();
        let other_receiver_address = other_receiver_wallet_secret
            .nth_generation_spending_key(0)
            .to_address();

        // 1. Create new block 1 and store it to the DB
        let genesis_block = Block::genesis_block(network);
        let launch = genesis_block.kernel.header.timestamp;
        let seven_months = Timestamp::months(7);
        let (mock_block_1a, _, _) =
            make_mock_block(&genesis_block, None, other_receiver_address, rng.gen());
        {
            global_state
                .chain
                .archival_state_mut()
                .write_block_as_tip(&mock_block_1a)
                .await?;
        }

        // Verify that wallet has a monitored UTXO (from genesis)
        let wallet_status = global_state.get_wallet_status_for_tip().await;
        assert!(!wallet_status
            .synced_unspent_available_amount(launch + seven_months)
            .is_zero());

        // Verify that this is unsynced with mock_block_1a
        assert!(
            global_state
                .wallet_state
                .is_synced_to(genesis_block.hash())
                .await
        );
        assert!(
            !global_state
                .wallet_state
                .is_synced_to(mock_block_1a.hash())
                .await
        );

        // Call resync
        global_state
            .resync_membership_proofs_from_stored_blocks(mock_block_1a.hash())
            .await
            .unwrap();

        // Verify that it is synced
        assert!(
            global_state
                .wallet_state
                .is_synced_to(mock_block_1a.hash())
                .await
        );

        // Verify that MPs are valid
        assert!(
            wallet_state_has_all_valid_mps_for(&global_state.wallet_state, &mock_block_1a).await
        );

        Ok(())
    }

    #[traced_test]
    #[tokio::test]
    async fn resync_ms_membership_proofs_fork_test() -> Result<()> {
        let mut rng = thread_rng();
        let network = Network::RegTest;
        let global_state_lock =
            mock_genesis_global_state(network, 2, WalletSecret::devnet_wallet()).await;
        let mut global_state = global_state_lock.lock_guard_mut().await;
        let own_spending_key = global_state
            .wallet_state
            .wallet_secret
            .nth_generation_spending_key(0);
        let own_receiving_address = own_spending_key.to_address();

        // 1. Create new block 1a where we receive a coinbase UTXO, store it
        let genesis_block = global_state.chain.archival_state().get_tip().await;
        let (mock_block_1a, coinbase_utxo, coinbase_output_randomness) =
            make_mock_block(&genesis_block, None, own_receiving_address, rng.gen());
        global_state
            .set_new_self_mined_tip(
                mock_block_1a.clone(),
                ExpectedUtxo::new(
                    coinbase_utxo,
                    coinbase_output_randomness,
                    own_spending_key.privacy_preimage,
                    UtxoNotifier::OwnMiner,
                ),
            )
            .await
            .unwrap();

        // Verify that wallet has monitored UTXOs, from genesis and from block_1a
        let wallet_status = global_state
            .wallet_state
            .get_wallet_status_from_lock(mock_block_1a.hash())
            .await;
        assert_eq!(2, wallet_status.synced_unspent.len());

        // Make a new fork from genesis that makes us lose the coinbase UTXO of block 1a
        let other_wallet_secret = WalletSecret::new_random();
        let other_receiving_address = other_wallet_secret
            .nth_generation_spending_key(0)
            .to_address();
        let mut parent_block = genesis_block;
        for _ in 0..5 {
            let (next_block, _, _) =
                make_mock_block(&parent_block, None, other_receiving_address, rng.gen());
            global_state.set_new_tip(next_block.clone()).await.unwrap();
            parent_block = next_block;
        }

        // Call resync which fails to sync the UTXO that was abandoned when block 1a was abandoned
        global_state
            .resync_membership_proofs_from_stored_blocks(parent_block.hash())
            .await
            .unwrap();

        // Verify that one MUTXO is unsynced, and that 1 (from genesis) is synced
        let wallet_status_after_forking = global_state
            .wallet_state
            .get_wallet_status_from_lock(parent_block.hash())
            .await;
        assert_eq!(1, wallet_status_after_forking.synced_unspent.len());
        assert_eq!(1, wallet_status_after_forking.unsynced_unspent.len());

        // Verify that the MUTXO from block 1a is considered abandoned, and that the one from
        // genesis block is not.
        let monitored_utxos = global_state.wallet_state.wallet_db.monitored_utxos();
        assert!(
            !monitored_utxos
                .get(0)
                .await
                .was_abandoned(parent_block.hash(), global_state.chain.archival_state())
                .await
        );
        assert!(
            monitored_utxos
                .get(1)
                .await
                .was_abandoned(parent_block.hash(), global_state.chain.archival_state())
                .await
        );

        Ok(())
    }

    #[traced_test]
    #[tokio::test]
    async fn resync_ms_membership_proofs_across_stale_fork() -> Result<()> {
        let mut rng = thread_rng();
        let network = Network::RegTest;
        let global_state_lock =
            mock_genesis_global_state(network, 2, WalletSecret::devnet_wallet()).await;
        let mut global_state = global_state_lock.lock_guard_mut().await;
        let wallet_secret = global_state.wallet_state.wallet_secret.clone();
        let own_spending_key = wallet_secret.nth_generation_spending_key(0);
        let own_receiving_address = own_spending_key.to_address();
        let other_wallet_secret = WalletSecret::new_random();
        let other_receiving_address = other_wallet_secret
            .nth_generation_spending_key(0)
            .to_address();

        // 1. Create new block 1a where we receive a coinbase UTXO, store it
        let genesis_block = global_state.chain.archival_state().get_tip().await;
        assert!(genesis_block.kernel.header.height.is_genesis());
        let (mock_block_1a, coinbase_utxo_1a, cb_utxo_output_randomness_1a) =
            make_mock_block(&genesis_block, None, own_receiving_address, rng.gen());
        {
            global_state
                .set_new_self_mined_tip(
                    mock_block_1a.clone(),
                    ExpectedUtxo::new(
                        coinbase_utxo_1a,
                        cb_utxo_output_randomness_1a,
                        own_spending_key.privacy_preimage,
                        UtxoNotifier::OwnMiner,
                    ),
                )
                .await
                .unwrap();

            // Verify that UTXO was recorded
            let wallet_status_after_1a = global_state
                .wallet_state
                .get_wallet_status_from_lock(mock_block_1a.hash())
                .await;
            assert_eq!(2, wallet_status_after_1a.synced_unspent.len());
        }

        // Add 100 blocks on top of 1a, *not* mined by us
        let mut fork_a_block = mock_block_1a.clone();
        for _ in 0..100 {
            let (next_a_block, _, _) =
                make_mock_block(&fork_a_block, None, other_receiving_address, rng.gen());
            global_state
                .set_new_tip(next_a_block.clone())
                .await
                .unwrap();
            fork_a_block = next_a_block;
        }

        // Verify that all both MUTXOs have synced MPs
        let wallet_status_on_a_fork = global_state
            .wallet_state
            .get_wallet_status_from_lock(fork_a_block.hash())
            .await;

        assert_eq!(2, wallet_status_on_a_fork.synced_unspent.len());

        // Fork away from the "a" chain to the "b" chain, with block 1a as LUCA
        let mut fork_b_block = mock_block_1a.clone();
        for _ in 0..100 {
            let (next_b_block, _, _) =
                make_mock_block(&fork_b_block, None, other_receiving_address, rng.gen());
            global_state
                .set_new_tip(next_b_block.clone())
                .await
                .unwrap();
            fork_b_block = next_b_block;
        }

        // Verify that there are zero MUTXOs with synced MPs
        let wallet_status_on_b_fork_before_resync = global_state
            .wallet_state
            .get_wallet_status_from_lock(fork_b_block.hash())
            .await;
        assert_eq!(
            0,
            wallet_status_on_b_fork_before_resync.synced_unspent.len()
        );
        assert_eq!(
            2,
            wallet_status_on_b_fork_before_resync.unsynced_unspent.len()
        );

        // Run the resync and verify that MPs are synced
        global_state
            .resync_membership_proofs_from_stored_blocks(fork_b_block.hash())
            .await
            .unwrap();
        let wallet_status_on_b_fork_after_resync = global_state
            .wallet_state
            .get_wallet_status_from_lock(fork_b_block.hash())
            .await;
        assert_eq!(2, wallet_status_on_b_fork_after_resync.synced_unspent.len());
        assert_eq!(
            0,
            wallet_status_on_b_fork_after_resync.unsynced_unspent.len()
        );

        // `wallet_state_has_all_valid_mps_for`
        // Make a new chain c with genesis block as LUCA. Verify that the genesis UTXO can be synced
        // to this new chain
        let mut fork_c_block = genesis_block.clone();
        for _ in 0..100 {
            let (next_c_block, _, _) =
                make_mock_block(&fork_c_block, None, other_receiving_address, rng.gen());
            global_state
                .set_new_tip(next_c_block.clone())
                .await
                .unwrap();
            fork_c_block = next_c_block;
        }

        // Verify that there are zero MUTXOs with synced MPs
        let wallet_status_on_c_fork_before_resync = global_state
            .wallet_state
            .get_wallet_status_from_lock(fork_c_block.hash())
            .await;
        assert_eq!(
            0,
            wallet_status_on_c_fork_before_resync.synced_unspent.len()
        );
        assert_eq!(
            2,
            wallet_status_on_c_fork_before_resync.unsynced_unspent.len()
        );

        // Run the resync and verify that UTXO from genesis is synced, but that
        // UTXO from 1a is not synced.
        global_state
            .resync_membership_proofs_from_stored_blocks(fork_c_block.hash())
            .await
            .unwrap();
        let wallet_status_on_c_fork_after_resync = global_state
            .wallet_state
            .get_wallet_status_from_lock(fork_c_block.hash())
            .await;
        assert_eq!(1, wallet_status_on_c_fork_after_resync.synced_unspent.len());
        assert_eq!(
            1,
            wallet_status_on_c_fork_after_resync.unsynced_unspent.len()
        );

        // Also check that UTXO from 1a is considered abandoned
        let monitored_utxos = global_state.wallet_state.wallet_db.monitored_utxos();
        assert!(
            !monitored_utxos
                .get(0)
                .await
                .was_abandoned(fork_c_block.hash(), global_state.chain.archival_state())
                .await
        );
        assert!(
            monitored_utxos
                .get(1)
                .await
                .was_abandoned(fork_c_block.hash(), global_state.chain.archival_state())
                .await
        );

        Ok(())
    }

    #[traced_test]
    #[tokio::test]
    async fn flaky_mutator_set_test() {
        // Test various parts of the state update when a block contains multiple inputs and outputs
        // Scenario: Three parties: Alice, Bob, and Premine Receiver, mine blocks and pass coins
        // around.

        let seed = {
            let mut rng: StdRng =
                SeedableRng::from_rng(thread_rng()).expect("failure lifting thread_rng to StdRng");
            let seed: [u8; 32] = rng.gen();
            // let seed = [
            //     0xf4, 0xc2, 0x1c, 0xd0, 0x5a, 0xac, 0x99, 0xe7, 0x3a, 0x1e, 0x29, 0x7f, 0x16, 0xc1,
            //     0x50, 0x5e, 0x1e, 0xd, 0x4b, 0x49, 0x51, 0x9c, 0x1b, 0xa0, 0x38, 0x3c, 0xd, 0x83, 0x29,
            //     0xdb, 0xab, 0xe2,
            // ];
            println!(
                "seed: [{}]",
                seed.iter().map(|h| format!("{:#x}", h)).join(", ")
            );
            seed
        };
        let mut rng: StdRng = SeedableRng::from_seed(seed);
        let network = Network::RegTest;

        let genesis_wallet_state =
            mock_genesis_wallet_state(WalletSecret::devnet_wallet(), network).await;
        let genesis_spending_key = genesis_wallet_state
            .wallet_secret
            .nth_generation_spending_key(0);
        let genesis_state_lock =
            mock_genesis_global_state(network, 3, genesis_wallet_state.wallet_secret).await;

        let wallet_secret_alice = WalletSecret::new_pseudorandom(rng.gen());
        let alice_spending_key = wallet_secret_alice.nth_generation_spending_key(0);
        let alice_state_lock = mock_genesis_global_state(network, 3, wallet_secret_alice).await;

        let wallet_secret_bob = WalletSecret::new_pseudorandom(rng.gen());
        let bob_spending_key = wallet_secret_bob.nth_generation_spending_key(0);
        let bob_state_lock = mock_genesis_global_state(network, 3, wallet_secret_bob).await;

        let genesis_block = Block::genesis_block(network);
        let launch = genesis_block.kernel.header.timestamp;
        let seven_months = Timestamp::months(7);

        let (mut block_1, cb_utxo, cb_output_randomness) = make_mock_block_with_valid_pow(
            &genesis_block,
            None,
            genesis_spending_key.to_address(),
            rng.gen(),
        );

        // Send two outputs each to Alice and Bob, from genesis receiver
        let fee = NeptuneCoins::one();
        let sender_randomness: Digest = rng.gen();
        let tx_outputs_for_alice = vec![
            TxOutput::fake_announcement(
                Utxo {
                    lock_script_hash: alice_spending_key.to_address().lock_script().hash(),
                    coins: NeptuneCoins::new(41).to_native_coins(),
                },
                sender_randomness,
                alice_spending_key.to_address().privacy_digest,
            ),
            TxOutput::fake_announcement(
                Utxo {
                    lock_script_hash: alice_spending_key.to_address().lock_script().hash(),
                    coins: NeptuneCoins::new(59).to_native_coins(),
                },
                sender_randomness,
                alice_spending_key.to_address().privacy_digest,
            ),
        ];

        // Two outputs for Bob
        let tx_outputs_for_bob = vec![
            TxOutput::fake_announcement(
                Utxo {
                    lock_script_hash: bob_spending_key.to_address().lock_script().hash(),
                    coins: NeptuneCoins::new(141).to_native_coins(),
                },
                sender_randomness,
                bob_spending_key.to_address().privacy_digest,
            ),
            TxOutput::fake_announcement(
                Utxo {
                    lock_script_hash: bob_spending_key.to_address().lock_script().hash(),
                    coins: NeptuneCoins::new(59).to_native_coins(),
                },
                sender_randomness,
                bob_spending_key.to_address().privacy_digest,
            ),
        ];
        {
            let (tx_to_alice_and_bob, expected_utxos_ab) = genesis_state_lock
                .lock_guard()
                .await
                .create_transaction_test_wrapper(
                    [tx_outputs_for_alice.clone(), tx_outputs_for_bob.clone()].concat(),
                    fee,
                    launch + seven_months,
                )
                .await
                .unwrap();

            // inform wallet of any expected utxos from this tx.
            genesis_state_lock
                .lock_guard_mut()
                .await
                .add_expected_utxos_to_wallet(expected_utxos_ab)
                .await
                .unwrap();

            // Absorb and verify validity
            block_1
                .accumulate_transaction(
                    tx_to_alice_and_bob,
                    &genesis_block.kernel.body.mutator_set_accumulator,
                )
                .await;
            let now = genesis_block.kernel.header.timestamp;
            assert!(block_1.is_valid(&genesis_block, now + seven_months));
        }

        println!("Accumulated transaction into block_1.");
        println!(
            "Transaction has {} inputs (removal records) and {} outputs (addition records)",
            block_1.kernel.body.transaction.kernel.inputs.len(),
            block_1.kernel.body.transaction.kernel.outputs.len()
        );

        // Update states with `block_1`
        for rec_data in tx_outputs_for_alice {
            alice_state_lock
                .lock_guard_mut()
                .await
                .wallet_state
                .expected_utxos
                .add_expected_utxo(
                    rec_data.utxo.clone(),
                    rec_data.sender_randomness,
                    alice_spending_key.privacy_preimage,
                    UtxoNotifier::Cli,
                )
                .unwrap();
        }

        for rec_data in tx_outputs_for_bob {
            bob_state_lock
                .lock_guard_mut()
                .await
                .wallet_state
                .expected_utxos
                .add_expected_utxo(
                    rec_data.utxo.clone(),
                    rec_data.sender_randomness,
                    bob_spending_key.privacy_preimage,
                    UtxoNotifier::Cli,
                )
                .unwrap();
        }

        genesis_state_lock
            .lock_guard_mut()
            .await
            .set_new_self_mined_tip(
                block_1.clone(),
                ExpectedUtxo::new(
                    cb_utxo,
                    cb_output_randomness,
                    genesis_spending_key.privacy_preimage,
                    UtxoNotifier::OwnMiner,
                ),
            )
            .await
            .unwrap();

        for state_lock in [&alice_state_lock, &bob_state_lock] {
            let mut state = state_lock.lock_guard_mut().await;
            state.set_new_tip(block_1.clone()).await.unwrap();
        }

        assert_eq!(
                3,
                genesis_state_lock
                    .lock_guard_mut()
                    .await
                    .wallet_state
                    .wallet_db
                    .monitored_utxos()
                    .len().await, "Genesis receiver must have 3 UTXOs after block 1: change from transaction, coinbase from block 1, and the spent premine UTXO"
            );

        // Now Alice should have a balance of 100 and Bob a balance of 200
        assert_eq!(
            NeptuneCoins::new(100),
            alice_state_lock
                .lock_guard()
                .await
                .get_wallet_status_for_tip()
                .await
                .synced_unspent_available_amount(launch + seven_months)
        );
        assert_eq!(
            NeptuneCoins::new(200),
            bob_state_lock
                .lock_guard()
                .await
                .get_wallet_status_for_tip()
                .await
                .synced_unspent_available_amount(launch + seven_months)
        );

        // Make two transactions: Alice sends two UTXOs to Genesis and Bob sends three UTXOs to genesis
        let tx_outputs_from_alice = vec![
            TxOutput::fake_announcement(
                Utxo {
                    lock_script_hash: genesis_spending_key.to_address().lock_script().hash(),
                    coins: NeptuneCoins::new(50).to_native_coins(),
                },
                rng.gen(),
                genesis_spending_key.to_address().privacy_digest,
            ),
            TxOutput::fake_announcement(
                Utxo {
                    lock_script_hash: genesis_spending_key.to_address().lock_script().hash(),
                    coins: NeptuneCoins::new(49).to_native_coins(),
                },
                rng.gen(),
                genesis_spending_key.to_address().privacy_digest,
            ),
        ];
        let now = genesis_block.kernel.header.timestamp;
        let (tx_from_alice, expected_utxos_alice) = alice_state_lock
            .lock_guard()
            .await
            .create_transaction_test_wrapper(
                tx_outputs_from_alice.clone(),
                NeptuneCoins::new(1),
                now,
            )
            .await
            .unwrap();

        // inform wallet of any expected utxos from this tx.
        alice_state_lock
            .lock_guard_mut()
            .await
            .add_expected_utxos_to_wallet(expected_utxos_alice)
            .await
            .unwrap();

        let tx_outputs_from_bob = vec![
            TxOutput::fake_announcement(
                Utxo {
                    lock_script_hash: genesis_spending_key.to_address().lock_script().hash(),
                    coins: NeptuneCoins::new(50).to_native_coins(),
                },
                rng.gen(),
                genesis_spending_key.to_address().privacy_digest,
            ),
            TxOutput::fake_announcement(
                Utxo {
                    lock_script_hash: genesis_spending_key.to_address().lock_script().hash(),
                    coins: NeptuneCoins::new(50).to_native_coins(),
                },
                rng.gen(),
                genesis_spending_key.to_address().privacy_digest,
            ),
            TxOutput::fake_announcement(
                Utxo {
                    lock_script_hash: genesis_spending_key.to_address().lock_script().hash(),
                    coins: NeptuneCoins::new(98).to_native_coins(),
                },
                rng.gen(),
                genesis_spending_key.to_address().privacy_digest,
            ),
        ];
        let (tx_from_bob, expected_utxos_bob) = bob_state_lock
            .lock_guard()
            .await
            .create_transaction_test_wrapper(tx_outputs_from_bob.clone(), NeptuneCoins::new(2), now)
            .await
            .unwrap();

        // inform wallet of any expected utxos from this tx.
        bob_state_lock
            .lock_guard_mut()
            .await
            .add_expected_utxos_to_wallet(expected_utxos_bob)
            .await
            .unwrap();

        // Make block_2 with tx that contains:
        // - 4 inputs: 2 from Alice and 2 from Bob
        // - 6 outputs: 2 from Alice to Genesis, 3 from Bob to Genesis, and 1 coinbase to Genesis
        let (mut block_2, _cb_utxo_block_2, _cb_sender_randomness_block_2) =
            make_mock_block_with_valid_pow(
                &block_1,
                None,
                genesis_spending_key.to_address(),
                rng.gen(),
            );
        block_2
            .accumulate_transaction(tx_from_alice, &block_1.kernel.body.mutator_set_accumulator)
            .await;
        assert_eq!(2, block_2.kernel.body.transaction.kernel.inputs.len());
        assert_eq!(3, block_2.kernel.body.transaction.kernel.outputs.len());

        block_2
            .accumulate_transaction(tx_from_bob, &block_1.kernel.body.mutator_set_accumulator)
            .await;
    }

    #[traced_test]
    #[tokio::test]
    async fn mock_global_state_is_valid() {
        let mut rng = thread_rng();
        let network = Network::RegTest;
        let global_state_lock =
            mock_genesis_global_state(network, 2, WalletSecret::devnet_wallet()).await;
        let mut global_state = global_state_lock.lock_guard_mut().await;
        let genesis_block = Block::genesis_block(network);
        let now = genesis_block.kernel.header.timestamp;

        let wallet_secret = WalletSecret::new_random();
        let receiving_address = wallet_secret.nth_generation_spending_key(0).to_address();
        let (block_1, _cb_utxo, _cb_output_randomness) =
            make_mock_block_with_valid_pow(&genesis_block, None, receiving_address, rng.gen());

        global_state.set_new_tip(block_1).await.unwrap();

        assert!(global_state
            .chain
            .light_state()
            .is_valid(&genesis_block, now));
    }
}
