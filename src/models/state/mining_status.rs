use std::fmt::Display;
use std::time::Duration;
use std::time::SystemTime;

use itertools::Itertools;
use serde::Deserialize;
use serde::Serialize;

use crate::models::blockchain::block::Block;
use crate::models::blockchain::type_scripts::native_currency_amount::NativeCurrencyAmount;

const MIN_CONNECTIONS_FOR_MINING: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuessingWorkInfo {
    work_start: SystemTime,
    num_inputs: usize,
    num_outputs: usize,
    total_coinbase: NativeCurrencyAmount,
    total_guesser_fee: NativeCurrencyAmount,
}

#[cfg(test)]
impl Default for GuessingWorkInfo {
    fn default() -> Self {
        Self {
            work_start: SystemTime::now(),
            num_inputs: 1,
            num_outputs: 2,
            total_coinbase: NativeCurrencyAmount::coins(128),
            total_guesser_fee: NativeCurrencyAmount::coins(120),
        }
    }
}

impl From<&Block> for GuessingWorkInfo {
    fn from(block: &Block) -> Self {
        Self::new(block)
    }
}

impl GuessingWorkInfo {
    pub(crate) fn new(block: &Block) -> Self {
        Self {
            work_start: SystemTime::now(),
            num_inputs: block.body().transaction_kernel.inputs.len(),
            num_outputs: block.body().transaction_kernel.outputs.len(),
            total_coinbase: block.body().transaction_kernel.coinbase.unwrap_or_default(),
            total_guesser_fee: block.body().transaction_kernel.fee,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MiningState {
    Disabled = 0,
    Init = 1,
    Paused = 2, // ByRpc, SyncBlocks, AwaitConnections
    AwaitBlockProposal = 3,
    AwaitBlock = 4,
    Composing = 5,
    Guessing = 6,
    NewTipBlock = 7,
    ComposeError = 8,
    Shutdown = 9,
}

#[rustfmt::skip]
const MINING_STATE_TRANSITIONS: [&[MiningState]; 10] = [
    // MiningState::Disabled
    &[],

    // MiningState::Init
    &[
        MiningState::Paused,
        MiningState::AwaitBlockProposal,
        MiningState::AwaitBlock,
        MiningState::Shutdown,
    ],

    // MiningState::Paused
    &[
        MiningState::Init,
        MiningState::Shutdown
    ],

    // MiningState::AwaitBlockProposal
    &[
        MiningState::Composing,
        MiningState::AwaitBlock,
        MiningState::Paused,
        MiningState::Shutdown,
    ],

    // MiningState::AwaitBlock
    &[
        MiningState::Guessing,
        MiningState::NewTipBlock,
        MiningState::Paused,
        MiningState::Shutdown,
    ],

    // MiningState::Composing
    &[
        MiningState::AwaitBlock,
        MiningState::Paused,
        MiningState::ComposeError,
        MiningState::Shutdown,
    ],

    // MiningState::Guessing
    &[
        MiningState::NewTipBlock,
        MiningState::Paused,
        MiningState::Shutdown,
    ],

    // MiningState::NewTipBlock
    &[
        MiningState::Init,
        MiningState::Paused,
        MiningState::Shutdown,
    ],

    // MiningState::ComposeError
    &[
        MiningState::Shutdown
    ],

    // MiningState::Shutdown
    &[],
];

pub struct MiningStateMachine {
    status: MiningStatus, // holds a MiningState.

    syncing: bool,
    paused_by_rpc: bool,
    connections: u32,

    role_compose: bool,
    role_guess: bool,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid state transition from {:?} to {:?}", old_state, new_state)]
pub struct InvalidStateTransition {
    pub old_state: MiningState,
    pub new_state: MiningState,
}

impl MiningStateMachine {
    pub fn new(role_compose: bool, role_guess: bool) -> Self {
        Self {
            status: MiningStatus::init(),
            syncing: false,
            paused_by_rpc: false,
            connections: 0,
            role_compose,
            role_guess,
        }
    }

    pub fn mining_status(&self) -> &MiningStatus {
        &self.status
    }

    pub fn try_advance(&mut self, new_status: MiningStatus) -> Result<(), InvalidStateTransition> {
        tracing::debug!(
            "try_advance: old_state: {}, new_state: {}",
            self.status.name(),
            new_status.name()
        );

        self.ensure_allowed(&new_status)?;

        // special handling for pause.
        if let MiningStatus::Paused(ref reasons) = new_status {
            assert!(!reasons.is_empty());
            for reason in reasons {
                self.pause(reason)
            }
        } else {
            self.set_new_status(new_status);
        }

        Ok(())
    }

    pub fn exec_states(&mut self, states: Vec<MiningStatus>) -> Result<(), InvalidStateTransition> {
            for state in states {
                self.try_advance(state)?
            }
            Ok(())
    }    

    fn set_new_status(&mut self, new_status: MiningStatus) {
        self.status = new_status;
        tracing::debug!("set new state: {}", self.status.name());
    }

    pub fn set_connections(&mut self, connections: u32) {
        if connections < MIN_CONNECTIONS_FOR_MINING {
            let reason = MiningPausedReason::await_connections();
            let new_status = MiningStatus::paused(reason);
            if self.allowed(&new_status) {
                self.merge_set_paused_status(new_status);
            }
        } else if self.connections < MIN_CONNECTIONS_FOR_MINING {
            let new_status = MiningStatus::init();
            if self.allowed(&new_status) {
                self.set_new_status(new_status);
            } else {
                // connections was fine before, and still fine.
                // keep our existing state.
            }
        }
        self.connections = connections;
    }

    fn merge_set_paused_status(&mut self, new_status: MiningStatus) {
        let merged_status = match (self.status.clone(), new_status) {
            (MiningStatus::Paused(mut old_reasons), MiningStatus::Paused(mut new_reasons)) => {
                // todo: ensure unique
                old_reasons.append(&mut new_reasons);
                MiningStatus::Paused(old_reasons)
            }
            (_, MiningStatus::Paused(reasons)) => MiningStatus::Paused(reasons),
            _ => panic!("attempted to merge status other than Paused"),
        };
        self.set_new_status(merged_status);
    }

    pub fn pause(&mut self, reason: &MiningPausedReason) {
        match reason {
            MiningPausedReason::Rpc(_) => self.pause_by_rpc(),
            MiningPausedReason::SyncBlocks(_) => self.start_syncing(),
            MiningPausedReason::AwaitConnections(_) => self.set_connections(0),
        };
    }

    pub fn pause_by_rpc(&mut self) {
        let reason = MiningPausedReason::rpc();
        let new_status = MiningStatus::paused(reason);
        if self.allowed(&new_status) {
            self.merge_set_paused_status(new_status);
        }
        self.paused_by_rpc = true;
    }

    pub fn unpause_by_rpc(&mut self) {
        let new_status = MiningStatus::init();
        if self.allowed(&new_status) {
            self.set_new_status(new_status);
        }
        self.paused_by_rpc = false;
    }

    pub fn set_syncing(&mut self, syncing: bool) {
        if self.syncing != syncing {
            if syncing {
                self.stop_syncing()
            } else {
                self.start_syncing()
            }
        }
    }

    pub fn start_syncing(&mut self) {
        let reason = MiningPausedReason::sync_blocks();
        let new_status = MiningStatus::paused(reason);
        if self.allowed(&new_status) {
            self.merge_set_paused_status(new_status);
        }
        self.syncing = true;
    }

    pub fn stop_syncing(&mut self) {
        let new_status = MiningStatus::init();
        if self.allowed(&new_status) {
            self.set_new_status(new_status);
        }
        self.syncing = false;
    }

    pub fn allowed(&self, status: &MiningStatus) -> bool {
        let state = status.state();

        if *status == self.status {
            true
        } else if !self.mining_enabled() {
            state == MiningState::Disabled
        } else if self.paused_count() > 1 {
            state == MiningState::Shutdown  
        } else if !self.role_compose && state == MiningState::Composing {
            false
        } else if !self.role_guess && state == MiningState::Guessing {
            false
        } else {
            let state = status.state();
            let allowed_states: &[MiningState] =
                MINING_STATE_TRANSITIONS[self.status.state() as usize];
            allowed_states.iter().any(|v| *v == state)
        }
    }

    fn paused_count(&self) -> u8 {
        self.paused_by_rpc as u8
            + self.syncing as u8
            + (self.connections < MIN_CONNECTIONS_FOR_MINING) as u8
    }

    fn ensure_allowed(&self, new_status: &MiningStatus) -> Result<(), InvalidStateTransition> {
        if self.allowed(new_status) {
            Ok(())
        } else {
            Err(InvalidStateTransition {
                old_state: self.status.state(),
                new_state: new_status.state(),
            })
        }
    }

    pub(crate) fn mining_enabled(&self) -> bool {
        self.role_compose || self.role_guess
    }

    // pub(crate) fn mining_paused(&self) -> bool {
    //     self.paused_count() > 0
    // }

    pub(crate) fn can_start_guessing(&self) -> bool {
        self.role_guess && self.status.state() == MiningState::AwaitBlock
    }

    pub(crate) fn can_guess(&self) -> bool {
        self.role_guess && self.status.state() == MiningState::Guessing
    }

    pub(crate) fn can_start_composing(&self) -> bool {
        self.role_compose && self.status.state() == MiningState::AwaitBlockProposal
    }

    pub(crate) fn can_compose(&self) -> bool {
        self.role_compose && self.status.state() == MiningState::Composing
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MiningPausedReason {
    /// paused by rpc. (user)
    Rpc(SystemTime),

    /// syncing blocks
    SyncBlocks(SystemTime),

    /// await peer connections
    AwaitConnections(SystemTime),
}

impl MiningPausedReason {
    pub(crate) fn rpc() -> Self {
        Self::Rpc(SystemTime::now())
    }

    pub(crate) fn sync_blocks() -> Self {
        Self::SyncBlocks(SystemTime::now())
    }

    pub(crate) fn await_connections() -> Self {
        Self::AwaitConnections(SystemTime::now())
    }

    pub fn since(&self) -> SystemTime {
        match *self {
            Self::Rpc(i) => i,
            Self::SyncBlocks(i) => i,
            Self::AwaitConnections(i) => i,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Rpc(_) => "user",
            Self::SyncBlocks(_) => "syncing blocks",
            Self::AwaitConnections(_) => "await connections",
        }
    }
}

impl Display for MiningPausedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let desc = self.description();
        let elapsed = self.since().elapsed();
        write!(f, "{} for {}", desc, human_duration_secs(&elapsed))
    }
}

// impl Display for MiningPausedReasons {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         write!(f, "{}", self.join(", "))
//     }
// }

/// normal operation state transitions:
///
/// Guessing  --> Inactive(AwaitBlockProposal)
/// Composing --> Inactive(AwaitBlock)
///
/// when node is composing and guessing:
///      Composing --> Inactive(AwaitBlock) --> Guessing --> Inactive(AwaitBlockProposal) --> Composing ...
///
/// when node is composing only:
///      Composing --> Inactive(AwaitBlock) --> Composing ...
///
/// when node is guessing only:
///      Guessing --> Inactive(AwaitBlockProposal) --> Guessing ...
///
/// Disabled --> none.  (final)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MiningStatus {
    Disabled(SystemTime),
    Init(SystemTime),
    Paused(Vec<MiningPausedReason>), // ByRpc, SyncBlocks, AwaitConnections
    AwaitBlockProposal(SystemTime),
    AwaitBlock(SystemTime),
    Composing(SystemTime),
    Guessing(GuessingWorkInfo),
    NewTipBlock(SystemTime),
    ComposeError(SystemTime),
    Shutdown(SystemTime),
}

impl MiningStatus {
    pub fn disabled() -> Self {
        Self::Disabled(SystemTime::now())
    }

    pub fn init() -> Self {
        Self::Init(SystemTime::now())
    }

    pub fn paused(reason: MiningPausedReason) -> Self {
        Self::Paused(vec![reason])
    }

    pub fn await_block_proposal() -> Self {
        Self::AwaitBlockProposal(SystemTime::now())
    }

    pub fn await_block() -> Self {
        Self::AwaitBlock(SystemTime::now())
    }

    pub fn composing() -> Self {
        Self::Composing(SystemTime::now())
    }

    pub fn guessing(work_info: GuessingWorkInfo) -> Self {
        Self::Guessing(work_info)
    }

    pub fn new_tip_block() -> Self {
        Self::NewTipBlock(SystemTime::now())
    }

    pub fn compose_error() -> Self {
        Self::ComposeError(SystemTime::now())
    }

    pub fn shutdown() -> Self {
        Self::Shutdown(SystemTime::now())
    }

    pub fn is_disabled(&self) -> bool {
        self.state() == MiningState::Disabled
    }

    pub fn is_init(&self) -> bool {
        self.state() == MiningState::Init
    }

    pub fn is_paused(&self) -> bool {
        self.state() == MiningState::Paused
    }

    pub fn is_await_block_proposal(&self) -> bool {
        self.state() == MiningState::AwaitBlockProposal
    }

    pub fn is_await_block(&self) -> bool {
        self.state() == MiningState::AwaitBlock
    }

    pub fn is_composing(&self) -> bool {
        self.state() == MiningState::Composing
    }

    pub fn is_guessing(&self) -> bool {
        self.state() == MiningState::Guessing
    }

    pub fn is_new_tip_block(&self) -> bool {
        self.state() == MiningState::NewTipBlock
    }

    pub fn is_compose_error(&self) -> bool {
        self.state() == MiningState::ComposeError
    }

    pub fn is_shutdown(&self) -> bool {
        self.state() == MiningState::Shutdown
    }

    pub fn state(&self) -> MiningState {
        match *self {
            Self::Disabled(_) => MiningState::Disabled,
            Self::Init(_) => MiningState::Init,
            Self::Paused(_) => MiningState::Paused,
            Self::AwaitBlockProposal(_) => MiningState::AwaitBlockProposal,
            Self::AwaitBlock(_) => MiningState::AwaitBlock,
            Self::Composing(_) => MiningState::Composing,
            Self::Guessing(_) => MiningState::Guessing,
            Self::NewTipBlock(_) => MiningState::NewTipBlock,
            Self::ComposeError(_) => MiningState::ComposeError,
            Self::Shutdown(_) => MiningState::Shutdown,
        }
    }

    pub(crate) fn name(&self) -> &str {
        match *self {
            Self::Disabled(_) => "disabled",
            Self::Init(_) => "initializing",
            Self::Paused(_) => "paused",
            Self::AwaitBlockProposal(_) => "await block proposal",
            Self::AwaitBlock(_) => "await block",
            Self::Composing(_) => "composing",
            Self::Guessing(_) => "guessing",
            Self::NewTipBlock(_) => "new tip block",
            Self::ComposeError(_) => "composer error",
            Self::Shutdown(_) => "shutdown",
        }
    }

    pub fn since(&self) -> SystemTime {
        match *self {
            Self::Disabled(t) => t,
            Self::Init(t) => t,
            Self::Paused(ref reasons) => reasons.iter().map(|r| r.since()).min().unwrap(),
            Self::AwaitBlockProposal(t) => t,
            Self::AwaitBlock(t) => t,
            Self::Composing(t) => t,
            Self::Guessing(w) => w.work_start,
            Self::NewTipBlock(t) => t,
            Self::ComposeError(t) => t,
            Self::Shutdown(t) => t,
        }
    }

    // pub(crate) fn paused_reasons(&self) -> &[MiningPausedReason] {
    //     match self {
    //         Self::Paused(reasons) => reasons,
    //         _ => &[],
    //     }
    // }
}

impl Display for MiningStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let input_output_info = match self {
            MiningStatus::Guessing(info) => {
                format!(" {}/{}", info.num_inputs, info.num_outputs)
            }
            _ => String::default(),
        };

        let work_type_and_duration = match self {
            MiningStatus::Disabled(_) => self.name().to_string(),
            MiningStatus::Paused(reasons) => {
                format!(
                    "paused for {}  ({})",
                    human_duration_secs(&self.since().elapsed()),
                    reasons.iter().map(|r| r.description()).join(", ")
                )
            }
            _ => format!(
                "{} for {}",
                self.name(),
                human_duration_secs(&self.since().elapsed()),
            ),
        };
        let reward = match self {
            MiningStatus::Guessing(block_work_info) => format!(
                "; total guesser reward: {}",
                block_work_info.total_guesser_fee
            ),
            _ => String::default(),
        };

        write!(f, "{work_type_and_duration}{input_output_info}{reward}",)
    }
}

// formats a duration in human readable form, to seconds precision.
// eg: 7h 5m 23s
fn human_duration_secs(duration_exact: &Result<Duration, std::time::SystemTimeError>) -> String {
    // remove sub-second component, so humantime ends with seconds.
    // also set to 0 if any error.
    let duration_to_secs = duration_exact
        .as_ref()
        .map(|v| *v - Duration::from_nanos(v.subsec_nanos().into()))
        .unwrap_or(Duration::ZERO);
    humantime::format_duration(duration_to_secs).to_string()
}


#[cfg(test)]
mod state_machine_tests {

    use super::*;

    #[test]
    fn compose_and_guess_happy_path() -> anyhow::Result<()> {

        let mut machine = MiningStateMachine::new(true, true);
        machine.exec_states(worker::compose_and_guess_happy_path())?;

        let mut machine = MiningStateMachine::new(true, false);
        assert!(machine.exec_states(worker::compose_and_guess_happy_path()).is_err());

        let mut machine = MiningStateMachine::new(false, true);
        assert!(machine.exec_states(worker::compose_and_guess_happy_path()).is_err());

        Ok(())
    }

    #[test]
    fn compose_happy_path() -> anyhow::Result<()> {

        let mut machine = MiningStateMachine::new(true, false);
        machine.exec_states(worker::compose_happy_path())?;

        let mut machine = MiningStateMachine::new(false, true);
        assert!(machine.exec_states(worker::compose_happy_path()).is_err());

        Ok(())
    }

    #[test]
    fn guess_happy_path() -> anyhow::Result<()> {

        let mut machine = MiningStateMachine::new(false, true);
        machine.exec_states(worker::guess_happy_path())?;

        let mut machine = MiningStateMachine::new(true, false);
        assert!(machine.exec_states(worker::guess_happy_path()).is_err());

        Ok(())
    }    

    mod worker {
        use super::*;

        pub(super) fn compose_happy_path()-> Vec<MiningStatus> {
            vec![
                MiningStatus::await_block_proposal(),
                MiningStatus::composing(),
                MiningStatus::await_block(),
                MiningStatus::new_tip_block(),
                MiningStatus::init(),
            ]
        }

        pub(super) fn guess_happy_path()-> Vec<MiningStatus> {
            vec![
                MiningStatus::await_block(),
                MiningStatus::guessing(Default::default()),
                MiningStatus::new_tip_block(),
                MiningStatus::init(),
            ]
        }        

        pub(super) fn compose_and_guess_happy_path()-> Vec<MiningStatus> {
            vec![
                MiningStatus::await_block_proposal(),
                MiningStatus::composing(),
                MiningStatus::await_block(),
                MiningStatus::guessing(Default::default()),
                MiningStatus::new_tip_block(),
                MiningStatus::init(),
            ]
        }

    }

}
