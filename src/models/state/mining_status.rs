use std::fmt::Display;
use std::time::Duration;
use std::time::SystemTime;

use serde::Deserialize;
use serde::Serialize;

use crate::models::blockchain::block::Block;
use crate::models::blockchain::type_scripts::native_currency_amount::NativeCurrencyAmount;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
            MiningStatus::Inactive(reason) => format!("inactive: {}", reason),
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
