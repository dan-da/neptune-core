use std::fmt::Display;
use std::time::Duration;
use std::time::SystemTime;

use serde::Deserialize;
use serde::Serialize;

use crate::models::blockchain::block::Block;
use crate::models::blockchain::type_scripts::native_currency_amount::NativeCurrencyAmount;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuessingWorkInfo {
    work_start: SystemTime,
    num_inputs: usize,
    num_outputs: usize,
    total_coinbase: NativeCurrencyAmount,
    total_guesser_fee: NativeCurrencyAmount,
}

impl GuessingWorkInfo {
    pub(crate) fn new(work_start: SystemTime, block: &Block) -> Self {
        Self {
            work_start,
            num_inputs: block.body().transaction_kernel.inputs.len(),
            num_outputs: block.body().transaction_kernel.outputs.len(),
            total_coinbase: block.body().transaction_kernel.coinbase.unwrap_or_default(),
            total_guesser_fee: block.body().transaction_kernel.fee,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComposingWorkInfo {
    // Only this info is available at the beginning of the composition work.
    // The rest of the information will have to be read from the log.
    work_start: SystemTime,
}

impl ComposingWorkInfo {
    pub(crate) fn new(work_start: SystemTime) -> Self {
        Self { work_start }
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
    ComposeError = 7,
    ShutDown = 8,
}

#[rustfmt::skip]
const MINING_STATE_TRANSITIONS: [&[MiningState]; 9] = [
    // MiningState::Disabled
    &[],

    // MiningState::Init
    &[
        MiningState::Paused,
        MiningState::AwaitBlockProposal,
        MiningState::AwaitBlock,
        MiningState::ShutDown,
    ],

    // MiningState::Paused
    &[
        MiningState::Init,
        MiningState::ShutDown
    ],

    // MiningState::AwaitBlockProposal
    &[
        MiningState::Composing,
        MiningState::AwaitBlock,
        MiningState::Paused,
        MiningState::ShutDown,
    ],

    // MiningState::AwaitBlock
    &[
        MiningState::Guessing,
        MiningState::Paused,
        MiningState::ShutDown,
    ],

    // MiningState::Composing
    &[
        MiningState::AwaitBlock,
        MiningState::Paused,
        MiningState::ComposeError,
        MiningState::ShutDown,
    ],

    // MiningState::Guessing
    &[
        MiningState::Init,
        MiningState::Paused,
        MiningState::ShutDown,
    ],

    // MiningState::ComposeError
    &[
        MiningState::ShutDown
    ],

    // MiningState::ShutDown
    &[],
];

pub struct MiningStateMachine {
    state: MiningState,

    syncing: bool,
    paused_by_rpc: bool,
    connections: u32,

    role_compose: bool,
    role_guess: bool,
}

#[derive(Debug, Clone)]
pub struct InvalidStateTransition {
    pub old_state: MiningState,
    pub new_state: MiningState,
}

impl MiningStateMachine {
    pub fn new(role_compose: bool, role_guess: bool) -> Self {
        Self {
            state: MiningState::Init,
            syncing: false,
            paused_by_rpc: false,
            connections: 0,
            role_compose,
            role_guess,
        }
    }

    pub fn try_advance(&mut self, new_state: MiningState) -> Result<(), InvalidStateTransition> {
        self.ensure_allowed(new_state)?;
        self.state = new_state;
        Ok(())
    }

    pub fn set_connections(&mut self, connections: u32) {
        if connections < 2 {
            let new_state = MiningState::Paused;
            if self.allowed(new_state) {
                self.state = new_state;
                self.connections = connections;
            }
        } else {
            if self.connections < 2 {
                let new_state = MiningState::Init;
                if self.allowed(new_state) {
                    self.state = new_state;
                    self.connections = connections;
                }
            } else {
                // connections was fine before, and still fine.
                // keep our existing state.
                self.connections = connections;
            }
        }
    }

    pub fn pause_by_rpc(&mut self) {
        let new_state = MiningState::Paused;
        if self.allowed(new_state) {
            self.state = new_state;
            self.paused_by_rpc = true;
        }
    }

    pub fn unpause_by_rpc(&mut self) {
        let new_state = MiningState::Init;
        if self.allowed(new_state) {
            self.state = new_state;
            self.paused_by_rpc = false;
        }
    }

    pub fn start_syncing(&mut self) {
        let new_state = MiningState::Paused;
        if self.allowed(new_state) {
            self.state = new_state;
            self.syncing = true;
        }
    }

    pub fn stop_syncing(&mut self) {
        let new_state = MiningState::Init;
        if self.allowed(new_state) {
            self.state = new_state;
            self.syncing = false;
        }
    }

    pub fn allowed(&self, state: MiningState) -> bool {
        if state == self.state {
            true
        } else if !self.mining_enabled() {
            state == MiningState::Disabled
        } else {
            let allowed_states: &[MiningState] = MINING_STATE_TRANSITIONS[self.state as usize];
            allowed_states.iter().any(|v| *v == state)
        }
    }

    fn ensure_allowed(&self, new_state: MiningState) -> Result<(), InvalidStateTransition> {
        if self.allowed(new_state) {
            Ok(())
        } else {
            Err(InvalidStateTransition {
                old_state: self.state,
                new_state,
            })
        }
    }

    fn mining_enabled(&self) -> bool {
        self.role_compose && self.role_guess
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MiningInactiveReason {
    /// disabled
    /// set at startup if mining (composing & guessing) is disabled.
    Disabled(SystemTime),

    /// initializing
    Init(SystemTime),

    /// paused by user
    PausedByUser(SystemTime),

    /// synching blocks
    SyncBlocks(SystemTime),

    /// await block proposal (guesser)
    /// set when a new block is added
    AwaitBlockProposal(SystemTime),

    /// await block
    /// set when a block-proposal is generated or received.
    AwaitBlock(SystemTime),

    /// await peer connections
    AwaitConnections(SystemTime),

    /// a new block has been added to tip
    NewTipBlock(SystemTime),

    /// error while composing
    ComposeError(SystemTime),

    /// shutdown
    Shutdown(SystemTime),
}

impl MiningInactiveReason {
    pub(crate) fn can_compose(&self) -> bool {
        matches!(self, Self::AwaitBlockProposal(_))
    }

    pub(crate) fn can_guess(&self) -> bool {
        matches!(self, Self::AwaitBlock(_))
    }

    pub(crate) fn disabled() -> Self {
        Self::Disabled(SystemTime::now())
    }

    pub(crate) fn init() -> Self {
        Self::Init(SystemTime::now())
    }

    pub(crate) fn paused_by_user() -> Self {
        Self::PausedByUser(SystemTime::now())
    }

    pub(crate) fn sync_blocks() -> Self {
        Self::SyncBlocks(SystemTime::now())
    }

    pub(crate) fn await_block_proposal() -> Self {
        Self::AwaitBlockProposal(SystemTime::now())
    }

    pub(crate) fn await_block() -> Self {
        Self::AwaitBlock(SystemTime::now())
    }

    pub(crate) fn await_connections() -> Self {
        Self::AwaitConnections(SystemTime::now())
    }

    pub(crate) fn new_tip_block() -> Self {
        Self::NewTipBlock(SystemTime::now())
    }

    pub(crate) fn compose_error() -> Self {
        Self::ComposeError(SystemTime::now())
    }

    pub(crate) fn shutdown() -> Self {
        Self::Shutdown(SystemTime::now())
    }

    pub fn since(&self) -> SystemTime {
        match *self {
            Self::Disabled(i) => i,
            Self::Init(i) => i,
            Self::PausedByUser(i) => i,
            Self::SyncBlocks(i) => i,
            Self::AwaitBlockProposal(i) => i,
            Self::AwaitBlock(i) => i,
            Self::AwaitConnections(i) => i,
            Self::NewTipBlock(i) => i,
            Self::ComposeError(i) => i,
            Self::Shutdown(i) => i,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Disabled(_) => "disabled",
            Self::Init(_) => "initializing",
            Self::PausedByUser(_) => "paused by user",
            Self::SyncBlocks(_) => "syncing blocks",
            Self::AwaitBlockProposal(_) => "await block proposal",
            Self::AwaitBlock(_) => "await block",
            Self::AwaitConnections(_) => "await connections",
            Self::NewTipBlock(_) => "new tip block",
            Self::ComposeError(_) => "new tip block",
            Self::Shutdown(_) => "shutdown",
        }
    }
}

impl Display for MiningInactiveReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let desc = self.description();
        let elapsed = self.since().elapsed();
        write!(f, "{} for {}", desc, human_duration_secs(&elapsed))
    }
}

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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MiningStatus {
    /// guessing
    /// set when guessing starts
    Guessing(GuessingWorkInfo),

    /// composing
    /// set when composing starts
    Composing(ComposingWorkInfo),

    /// inactive
    Inactive(MiningInactiveReason),
}

impl MiningStatus {
    pub(crate) fn can_compose(&self) -> bool {
        match self {
            Self::Composing(_) => true,
            Self::Guessing(_) => false,
            Self::Inactive(reason) => reason.can_compose(),
        }
    }

    pub(crate) fn can_guess(&self) -> bool {
        match self {
            Self::Composing(_) => false,
            Self::Guessing(_) => true,
            Self::Inactive(reason) => reason.can_guess(),
        }
    }

    pub(crate) fn inactive_reason(&self) -> Option<MiningInactiveReason> {
        match *self {
            Self::Inactive(reason) => Some(reason),
            _ => None,
        }
    }
}

impl Display for MiningStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let elapsed_time = match self {
            MiningStatus::Guessing(guessing_work_info) => {
                Some(guessing_work_info.work_start.elapsed())
            }
            MiningStatus::Composing(composing_work_info) => {
                Some(composing_work_info.work_start.elapsed())
            }
            _ => None,
        };
        let input_output_info = match self {
            MiningStatus::Guessing(info) => {
                format!(" {}/{}", info.num_inputs, info.num_outputs)
            }
            _ => String::default(),
        };

        let work_type_and_duration = match self {
            MiningStatus::Guessing(_) => {
                format!(
                    "guessing for {}",
                    human_duration_secs(&elapsed_time.unwrap())
                )
            }
            MiningStatus::Composing(_) => {
                format!(
                    "composing for {}",
                    human_duration_secs(&elapsed_time.unwrap())
                )
            }
            MiningStatus::Inactive(reason)
                if matches!(reason, MiningInactiveReason::Disabled(_)) =>
            {
                format!("inactive  ({})", reason.description())
            }
            MiningStatus::Inactive(reason) => format!(
                "inactive for {}  ({})",
                human_duration_secs(&reason.since().elapsed()),
                reason.description()
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
