use std::fmt::Display;
use std::time::Duration;
use std::time::SystemTime;

use itertools::Itertools;
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

#[derive(Debug, Clone, Copy)]
pub enum MiningEvent {
    Advance,

    Init,

    PauseRpc,
    UnPauseRpc,

    PauseSyncBlock,
    UnPauseSyncBlock,

    PauseNoConnection,
    UnPauseNoConnection,

    NewBlockProposal,
    NewTipBlock,

    ComposeError,

    Shutdown,
}

impl Display for MiningEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MiningState {
    // ---- happy path ----
    Init = 0,
    AwaitBlockProposal = 1,
    Composing = 2,
    AwaitBlock = 3,
    Guessing = 4,
    NewTipBlock = 5,
    // ---- end happy path ----
    ComposeError = 6,
    Paused = 7, // Rpc, SyncBlocks, NeedConnections
    Disabled = 8,
    Shutdown = 9,
}

#[rustfmt::skip]
const MINING_STATE_TRANSITIONS: [&[MiningState]; 10] = [

    // ----- start happy path -----

    // MiningState::Init
    &[
        MiningState::AwaitBlockProposal,
        MiningState::NewTipBlock,
        MiningState::Paused,
        MiningState::Shutdown,
    ],

    // MiningState::AwaitBlockProposal
    &[
        MiningState::Composing,
        MiningState::Paused,
        MiningState::Shutdown,
        MiningState::AwaitBlockProposal,
        MiningState::NewTipBlock,
    ],

    // MiningState::Composing
    &[
        MiningState::AwaitBlock,
        MiningState::ComposeError,
        MiningState::Paused,
        MiningState::Shutdown,
        MiningState::NewTipBlock,
    ],

    // MiningState::AwaitBlock
    &[
        MiningState::Guessing,
        MiningState::Paused,
        MiningState::Shutdown,
        MiningState::NewTipBlock,
    ],

    // MiningState::Guessing
    &[
        MiningState::AwaitBlock,   // if a new block-proposal arrives
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

    // ---- end happy path ----

    // MiningState::ComposeError
    &[
        MiningState::Shutdown
    ],

    // MiningState::Paused
    &[
        MiningState::Init,
        MiningState::Shutdown
    ],

    // MiningState::Disabled
    &[],


    // MiningState::Shutdown
    &[],
];

#[rustfmt::skip]
const HAPPY_PATH_STATE_TRANSITIONS: &[MiningState] = &[
    MiningState::Init,
    MiningState::AwaitBlockProposal,
    MiningState::Composing,
    MiningState::AwaitBlock,
    MiningState::Guessing,
    MiningState::NewTipBlock,
];

pub struct MiningStateMachine {
    status: MiningStatus, // holds a MiningState.

    paused_while_syncing: bool,
    paused_by_rpc: bool,
    paused_need_connection: bool,

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
            paused_while_syncing: false,
            paused_by_rpc: false,
            paused_need_connection: false,
            role_compose,
            role_guess,
        }
    }

    pub fn mining_status(&self) -> &MiningStatus {
        &self.status
    }

    /// this is a shortcut for ::handle_event(MiningEvent::Advance)
    pub fn advance(&mut self) -> Result<(), InvalidStateTransition> {
        if let Some(state) = HAPPY_PATH_STATE_TRANSITIONS
            .iter()
            .cycle()
            .filter(|v| **v == self.status.state())
            .skip(1)
            .next()
        {
            let new_status =
                MiningStatus::try_from(*state).map_err(|_| InvalidStateTransition {
                    old_state: self.status.state(),
                    new_state: *state,
                })?;
            self.advance_to(new_status)?;

            // composer role skips over AwaitBlockProposal
            if self.role_compose && *state == MiningState::AwaitBlockProposal {
                self.advance()?;
            }
            // guesser role skips over AwaitBlock
            if self.role_guess && *state == MiningState::AwaitBlock {
                self.advance()?;
            }
            Ok(())
        } else {
            unreachable!();
        }
    }

    pub fn handle_event(&mut self, event: MiningEvent) -> Result<(), InvalidStateTransition> {
        tracing::debug!(
            "handle_event: old_state: {}, event: {}",
            self.status.name(),
            event,
        );

        match event {
            MiningEvent::Advance => self.advance()?,

            MiningEvent::Init => self.advance_to(MiningStatus::init())?,

            MiningEvent::PauseRpc => self.pause_by_rpc(),
            MiningEvent::UnPauseRpc => self.unpause_by_rpc(),

            MiningEvent::PauseSyncBlock => self.pause_while_syncing(),
            MiningEvent::UnPauseSyncBlock => self.unpause_while_syncing(),

            MiningEvent::PauseNoConnection => self.pause_need_connection(),
            MiningEvent::UnPauseNoConnection => self.unpause_need_connection(),

            MiningEvent::NewBlockProposal => self.advance_to(MiningStatus::await_block())?,
            MiningEvent::NewTipBlock => self.advance_to(MiningStatus::new_tip_block())?,

            MiningEvent::ComposeError => self.advance_to(MiningStatus::compose_error())?,

            MiningEvent::Shutdown => self.advance_to(MiningStatus::shutdown())?,
        }
        Ok(())
    }

    /// prefer advance() and handle_event() instead.
    pub fn advance_to(&mut self, new_status: MiningStatus) -> Result<(), InvalidStateTransition> {
        tracing::debug!(
            "advance_to: old_state: {}, new_state: {}",
            self.status.name(),
            new_status.name()
        );

        // special handling for pause.
        if let MiningStatus::Paused(ref reasons) = new_status {
            assert!(!reasons.is_empty());
            for reason in reasons {
                self.pause(reason)
            }
        } else {
            self.ensure_allowed(&new_status)?;
            self.set_new_status(new_status);
        }

        Ok(())
    }

    pub fn exec_states(&mut self, states: Vec<MiningStatus>) -> Result<(), InvalidStateTransition> {
        for state in states {
            self.advance_to(state)?
        }
        Ok(())
    }

    fn set_new_status(&mut self, new_status: MiningStatus) {
        self.status = new_status;
        tracing::debug!("set new state: {}", self.status.name());
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

    fn pause(&mut self, reason: &MiningPausedReason) {
        match reason {
            MiningPausedReason::Rpc(_) => self.pause_by_rpc(),
            MiningPausedReason::SyncBlocks(_) => self.pause_while_syncing(),
            MiningPausedReason::NeedConnections(_) => self.pause_need_connection(),
        };
    }

    /// shortcut for:
    ///
    /// if need_connection {
    ///   ::handle_event(MiningEvent::PauseNeedConnection)
    /// } else {
    ///   ::handle_event(MiningEvent::UnPauseNeedConnection)
    /// }
    pub fn set_need_connection(&mut self, need_connection: bool) {
        if self.paused_need_connection != need_connection {
            if need_connection {
                self.pause_need_connection()
            } else {
                self.unpause_need_connection()
            }
        }
    }

    fn pause_need_connection(&mut self) {
        let reason = MiningPausedReason::need_connections();
        let new_status = MiningStatus::paused(reason);
        if self.allowed(&new_status) {
            self.merge_set_paused_status(new_status);
        }
        self.paused_need_connection = true;
    }

    fn unpause_need_connection(&mut self) {
        let new_status = MiningStatus::init();
        if self.allowed(&new_status) {
            self.set_new_status(new_status);
        }
        self.paused_need_connection = false;
    }

    fn pause_by_rpc(&mut self) {
        let reason = MiningPausedReason::rpc();
        let new_status = MiningStatus::paused(reason);
        if self.allowed(&new_status) {
            self.merge_set_paused_status(new_status);
        }
        self.paused_by_rpc = true;
    }

    fn unpause_by_rpc(&mut self) {
        let new_status = MiningStatus::init();
        if self.allowed(&new_status) {
            self.set_new_status(new_status);
        }
        self.paused_by_rpc = false;
    }

    /// shortcut for:
    ///
    /// if sync_blocks {
    ///   ::handle_event(MiningEvent::PauseSyncBlocks)
    /// } else {
    ///   ::handle_event(MiningEvent::UnPauseSyncBlocks)
    /// }
    pub fn set_syncing(&mut self, syncing: bool) {
        if self.paused_while_syncing != syncing {
            if syncing {
                self.pause_while_syncing()
            } else {
                self.unpause_while_syncing()
            }
        }
    }

    fn pause_while_syncing(&mut self) {
        let reason = MiningPausedReason::sync_blocks();
        let new_status = MiningStatus::paused(reason);
        if self.allowed(&new_status) {
            self.merge_set_paused_status(new_status);
        }
        self.paused_while_syncing = true;
    }

    fn unpause_while_syncing(&mut self) {
        let new_status = MiningStatus::init();
        if self.allowed(&new_status) {
            self.set_new_status(new_status);
        }
        self.paused_while_syncing = false;
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
            + self.paused_while_syncing as u8
            + self.paused_need_connection as u8
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

    /// need peer connections
    NeedConnections(SystemTime),
}

impl MiningPausedReason {
    pub(crate) fn rpc() -> Self {
        Self::Rpc(SystemTime::now())
    }

    pub(crate) fn sync_blocks() -> Self {
        Self::SyncBlocks(SystemTime::now())
    }

    pub(crate) fn need_connections() -> Self {
        Self::NeedConnections(SystemTime::now())
    }

    pub fn since(&self) -> SystemTime {
        match *self {
            Self::Rpc(i) => i,
            Self::SyncBlocks(i) => i,
            Self::NeedConnections(i) => i,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Rpc(_) => "user",
            Self::SyncBlocks(_) => "syncing blocks",
            Self::NeedConnections(_) => "await connections",
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
    Paused(Vec<MiningPausedReason>), // ByRpc, SyncBlocks, NeedConnections
    AwaitBlockProposal(SystemTime),
    AwaitBlock(SystemTime),
    Composing(SystemTime),
    Guessing(GuessingWorkInfo),
    NewTipBlock(SystemTime),
    ComposeError(SystemTime),
    Shutdown(SystemTime),
}

impl TryFrom<MiningState> for MiningStatus {
    type Error = anyhow::Error; // todo: make a real error

    fn try_from(state: MiningState) -> anyhow::Result<Self> {
        Ok(match state {
            MiningState::Disabled => MiningStatus::disabled(),
            MiningState::Init => MiningStatus::init(),
            MiningState::AwaitBlockProposal => MiningStatus::await_block_proposal(),
            MiningState::AwaitBlock => MiningStatus::await_block(),
            MiningState::Composing => MiningStatus::composing(),
            MiningState::NewTipBlock => MiningStatus::new_tip_block(),
            MiningState::ComposeError => MiningStatus::compose_error(),
            MiningState::Shutdown => MiningStatus::shutdown(),
            _ => anyhow::bail!("cannot instantiate MiningStatus from {:?}", state),
        })
    }
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
        assert!(machine
            .exec_states(worker::compose_and_guess_happy_path())
            .is_err());

        let mut machine = MiningStateMachine::new(false, true);
        assert!(machine
            .exec_states(worker::compose_and_guess_happy_path())
            .is_err());

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

        pub(super) fn compose_happy_path() -> Vec<MiningStatus> {
            vec![
                MiningStatus::await_block_proposal(),
                MiningStatus::composing(),
                MiningStatus::await_block(),
                MiningStatus::new_tip_block(),
                MiningStatus::init(),
            ]
        }

        pub(super) fn guess_happy_path() -> Vec<MiningStatus> {
            vec![
                MiningStatus::await_block(),
                MiningStatus::guessing(Default::default()),
                MiningStatus::new_tip_block(),
                MiningStatus::init(),
            ]
        }

        pub(super) fn compose_and_guess_happy_path() -> Vec<MiningStatus> {
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
