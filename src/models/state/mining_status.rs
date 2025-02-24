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
    num_inputs: usize,
    num_outputs: usize,
    total_coinbase: NativeCurrencyAmount,
    total_guesser_fee: NativeCurrencyAmount,
}

impl From<&Block> for GuessingWorkInfo {
    fn from(block: &Block) -> Self {
        Self::new(block)
    }
}

impl GuessingWorkInfo {
    pub(crate) fn new(block: &Block) -> Self {
        Self {
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

    PauseNeedConnection,
    UnPauseNeedConnection,

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
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
    Paused = 7, // Rpc, SyncBlocks, NeedConnection
    Disabled = 8,
    Shutdown = 9,
}

impl MiningState {
    fn name(&self) -> &str {
        match *self {
            Self::Disabled => "disabled",
            Self::Init => "initializing",
            Self::Paused => "paused",
            Self::AwaitBlockProposal => "await block proposal",
            Self::AwaitBlock => "await block",
            Self::Composing => "composing",
            Self::Guessing => "guessing",
            Self::NewTipBlock => "new tip block",
            Self::ComposeError => "composer error",
            Self::Shutdown => "shutdown",
        }
    }
}

impl Display for MiningState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
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

    /// advances to next state in the happy path, taking role into account.
    ///
    /// this is equivalent to `::handle_event(MiningEvent::Advance)`
    ///
    /// important: this method should never be called when moving to the
    /// `Guessing` state. If so, the `Guessing` work-info will not be present.
    /// Instead use advance_to() and supply a `MiningStatus::Guessing(Some(_))`.
    pub fn advance(&mut self) -> Result<(), InvalidStateTransition> {
        let old_state = self.status.state();

        // finds happy-path state that is after our current state, if any.
        // cycles to beginning of happy-path if necessary.
        if let Some(state) = HAPPY_PATH_STATE_TRANSITIONS
            .iter()
            .circular_tuple_windows::<(_, _)>()
            .find(|(prev, _)| **prev == old_state)
            .map(|(_, next)| next)
        {
            let new_status = MiningStatus::from(*state);
            self.advance_to(new_status)?;

            // take role(s) into account (composer, guesser)
            match *state {
                // compose role skips over these 2 states
                MiningState::Guessing if self.role_compose => self.advance()?,
                MiningState::AwaitBlockProposal if self.role_compose => self.advance()?,

                // guess role skips over these 2 states
                MiningState::Composing if self.role_guess => self.advance()?,
                MiningState::AwaitBlock if self.role_guess => self.advance()?,
                _ => {}
            }

            Ok(())
        } else {
            unreachable!();
        }
    }

    /// handles an event.
    ///
    /// Some events have equivalent short-cut methods that can be called instead.
    ///
    /// the `Advance` event automatically moves to the next state in the happy-path.
    /// See `::advance()` for details.
    ///
    /// `::set_syncing()` can be used to pause/unpause because of SyncBlocks
    ///
    /// `::set_need_connection()` can be used to pause/unpause because of connection status.
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

            MiningEvent::PauseNeedConnection => self.pause_need_connection(),
            MiningEvent::UnPauseNeedConnection => self.unpause_need_connection(),

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
        if let MiningStatus::Paused(_, ref reasons) = new_status {
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

    #[cfg(test)]
    pub fn exec_states(&mut self, states: Vec<MiningStatus>) -> Result<(), InvalidStateTransition> {
        for state in states {
            self.advance_to(state)?
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn exec_events(&mut self, events: Vec<MiningEvent>) -> Result<(), InvalidStateTransition> {
        for event in events {
            self.handle_event(event)?
        }
        Ok(())
    }

    fn set_new_status(&mut self, new_status: MiningStatus) {
        self.status = new_status;
        tracing::debug!("set new state: {}", self.status.name());
    }

    fn merge_set_paused_status(&mut self, new_status: MiningStatus) {
        let merged_status = match (self.status.clone(), new_status) {
            (
                MiningStatus::Paused(old_time, mut old_reasons),
                MiningStatus::Paused(_, mut new_reasons),
            ) => {
                old_reasons.append(&mut new_reasons);
                // ensure unique
                MiningStatus::Paused(old_time, old_reasons.into_iter().unique().collect())
            }
            (_, MiningStatus::Paused(t, reasons)) => MiningStatus::Paused(t, reasons),
            _ => panic!("attempted to merge status other than Paused"),
        };
        self.set_new_status(merged_status);
    }

    fn pause(&mut self, reason: &MiningPausedReason) {
        match reason {
            MiningPausedReason::Rpc => self.pause_by_rpc(),
            MiningPausedReason::SyncBlocks => self.pause_while_syncing(),
            MiningPausedReason::NeedConnection => self.pause_need_connection(),
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
        let reason = MiningPausedReason::NeedConnection;
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
        let reason = MiningPausedReason::Rpc;
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
        let reason = MiningPausedReason::SyncBlocks;
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

        // we normally don't allow state equality since status variant data (eg
        // timestamps) can differ between 2 MiningStatus with same state.
        // We make an exception for Init because otherwise it can't be
        // manually set.
        if state == self.status.state() && state == MiningState::Init {
            true
        } else if *status == self.status {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, strum::EnumIter)]
pub enum MiningPausedReason {
    /// paused by rpc. (user)
    Rpc,

    /// syncing blocks
    SyncBlocks,

    /// need peer connections
    NeedConnection,
}

impl MiningPausedReason {
    pub fn description(&self) -> &str {
        match self {
            Self::Rpc => "user",
            Self::SyncBlocks => "syncing blocks",
            Self::NeedConnection => "await connections",
        }
    }
}

impl Display for MiningPausedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let desc = self.description();
        write!(f, "{}", desc)
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MiningStatus {
    Disabled(SystemTime),
    Init(SystemTime),
    Paused(SystemTime, Vec<MiningPausedReason>), // Rpc, SyncBlocks, NeedConnection
    AwaitBlockProposal(SystemTime),
    AwaitBlock(SystemTime),
    Composing(SystemTime),
    Guessing(SystemTime, Option<GuessingWorkInfo>),
    NewTipBlock(SystemTime),
    ComposeError(SystemTime),
    Shutdown(SystemTime),
}

impl From<MiningState> for MiningStatus {
    /// note that:
    ///
    ///   1. MiningStatus::Guessing will not have any work info.
    ///      It should only be used for unit-tests
    ///   2. MiningStatus::Paused will use MiningPausedReason::Rpc
    fn from(state: MiningState) -> Self {
        match state {
            MiningState::Disabled => MiningStatus::disabled(),
            MiningState::Init => MiningStatus::init(),
            MiningState::AwaitBlockProposal => MiningStatus::await_block_proposal(),
            MiningState::AwaitBlock => MiningStatus::await_block(),
            MiningState::Composing => MiningStatus::composing(),
            MiningState::Guessing => MiningStatus::Guessing(SystemTime::now(), None),
            MiningState::NewTipBlock => MiningStatus::new_tip_block(),
            MiningState::ComposeError => MiningStatus::compose_error(),
            MiningState::Shutdown => MiningStatus::shutdown(),
            MiningState::Paused => MiningStatus::paused(MiningPausedReason::Rpc),
        }
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
        Self::Paused(SystemTime::now(), vec![reason])
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

    pub fn guessing(work_info: Option<GuessingWorkInfo>) -> Self {
        Self::Guessing(SystemTime::now(), work_info)
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
            Self::Paused(..) => MiningState::Paused,
            Self::AwaitBlockProposal(_) => MiningState::AwaitBlockProposal,
            Self::AwaitBlock(_) => MiningState::AwaitBlock,
            Self::Composing(_) => MiningState::Composing,
            Self::Guessing(..) => MiningState::Guessing,
            Self::NewTipBlock(_) => MiningState::NewTipBlock,
            Self::ComposeError(_) => MiningState::ComposeError,
            Self::Shutdown(_) => MiningState::Shutdown,
        }
    }

    pub(crate) fn name(&self) -> String {
        self.state().name().to_owned()
    }

    pub fn since(&self) -> SystemTime {
        match *self {
            Self::Disabled(t) => t,
            Self::Init(t) => t,
            Self::Paused(t, _) => t,
            Self::AwaitBlockProposal(t) => t,
            Self::AwaitBlock(t) => t,
            Self::Composing(t) => t,
            Self::Guessing(t, _) => t,
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
            MiningStatus::Guessing(_, Some(info)) => {
                format!(" {}/{}", info.num_inputs, info.num_outputs)
            }
            _ => String::default(),
        };

        let work_type_and_duration = match self {
            MiningStatus::Disabled(_) => self.name().to_string(),
            MiningStatus::Paused(t, reasons) => {
                format!(
                    "paused for {}  ({})",
                    human_duration_secs(&t.elapsed()),
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
            MiningStatus::Guessing(_, Some(block_work_info)) => format!(
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

    const PAUSE_EVENTS: &[MiningEvent] = &[
        MiningEvent::PauseNeedConnection,
        MiningEvent::PauseRpc,
        MiningEvent::PauseSyncBlock,
    ];
    const UNPAUSE_EVENTS: &[MiningEvent] = &[
        MiningEvent::UnPauseNeedConnection,
        MiningEvent::UnPauseRpc,
        MiningEvent::UnPauseSyncBlock,
    ];

    mod states {
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
        fn can_pause_all_along_happy_path() -> anyhow::Result<()> {
            // test that all pause events can occur along happy path.
            for pause_event in PAUSE_EVENTS {
                worker::can_pause_all_along_happy_path(*pause_event)?;
            }
            Ok(())
        }

        #[test]
        fn can_pause_during_every_state() -> anyhow::Result<()> {
            // test that all pause events can occur during every state
            for pause_event in PAUSE_EVENTS {
                worker::can_pause_during_every_state(*pause_event)?;
            }
            Ok(())
        }

        #[test]
        fn pause_changes_only_certain_states() -> anyhow::Result<()> {
            // test that all pause events only change correct states
            for pause_event in PAUSE_EVENTS {
                worker::pause_changes_only_certain_states(*pause_event)?;
            }
            Ok(())
        }

        #[test]
        fn unpause_changes_only_certain_states() -> anyhow::Result<()> {
            // test that all pause events only change correct states
            for (pause_event, unpause_event) in worker::all_pause_and_unpause_events().into_iter() {
                worker::unpause_changes_only_certain_states(pause_event, unpause_event)?;
            }
            Ok(())
        }

        #[test]
        fn mixed_pause_unpause_types() -> anyhow::Result<()> {
            worker::mixed_pause_unpause_types()
        }

        mod worker {
            use rand::rng;
            use rand::seq::SliceRandom;
            use strum::IntoEnumIterator;

            use super::*;

            pub fn all_pause_and_unpause_events() -> Vec<(MiningEvent, MiningEvent)> {
                PAUSE_EVENTS
                    .into_iter()
                    .copied()
                    .zip(UNPAUSE_EVENTS.into_iter().copied())
                    .collect()
            }

            pub(super) fn compose_and_guess_happy_path() -> Vec<MiningStatus> {
                HAPPY_PATH_STATE_TRANSITIONS
                    .iter()
                    .cycle()
                    .take(HAPPY_PATH_STATE_TRANSITIONS.len() + 1)
                    .map(|s| MiningStatus::try_from(*s).unwrap())
                    .collect_vec()
            }

            pub(super) fn can_pause_during_every_state(
                pause_event: MiningEvent,
            ) -> anyhow::Result<()> {
                // for each state, we make a new state-machine and force it
                // to the target state, then pause it.
                let mut machine = MiningStateMachine::new(true, true);
                for status in all_status() {
                    machine.status = status;
                    machine.handle_event(pause_event)?;
                }
                Ok(())
            }

            pub(super) fn pause_changes_only_certain_states(
                pause_event: MiningEvent,
            ) -> anyhow::Result<()> {
                // for each state, we make a new state-machine and force it
                // to the target state, then pause it.
                let mut machine = MiningStateMachine::new(true, true);
                for status in all_status() {
                    machine.status = status.clone();
                    machine.handle_event(pause_event)?;

                    let ss = status.state();
                    let ms = machine.status.state();
                    let ps = MiningState::Paused;

                    // certain states should not switch to Paused state.
                    // (although the machine updates the appropiate pause
                    // flag internally)

                    match ss {
                        MiningState::Init => assert_eq!(ms, ps),
                        MiningState::AwaitBlockProposal => assert_eq!(ms, ps),
                        MiningState::Composing => assert_eq!(ms, ps),
                        MiningState::AwaitBlock => assert_eq!(ms, ps),
                        MiningState::Guessing => assert_eq!(ms, ps),
                        MiningState::NewTipBlock => assert_eq!(ms, ps),
                        MiningState::ComposeError => assert_eq!(ms, ss),
                        MiningState::Paused => assert_eq!(ms, ps),
                        MiningState::Disabled => assert_eq!(ms, ss),
                        MiningState::Shutdown => assert_eq!(ms, ss),
                    }
                }
                Ok(())
            }

            pub(super) fn unpause_changes_only_certain_states(
                pause_event: MiningEvent,
                unpause_event: MiningEvent,
            ) -> anyhow::Result<()> {
                // for each state, we make a new state-machine and force it
                // to the target state, then pause and unpause it.
                let mut machine = MiningStateMachine::new(true, true);
                for status in all_status() {
                    machine.status = status.clone();
                    machine.handle_event(pause_event)?;
                    machine.handle_event(unpause_event)?;

                    let ss = status.state();
                    let ms = machine.status.state();
                    let is = MiningState::Init;

                    // certain states should not switch state after UnPause
                    // (although the machine updates the appropiate pause
                    // flag internally)

                    match ss {
                        MiningState::Init => assert_eq!(ms, is),
                        MiningState::AwaitBlockProposal => assert_eq!(ms, is),
                        MiningState::Composing => assert_eq!(ms, is),
                        MiningState::AwaitBlock => assert_eq!(ms, is),
                        MiningState::Guessing => assert_eq!(ms, is),
                        MiningState::NewTipBlock => assert_eq!(ms, is),
                        MiningState::ComposeError => assert_eq!(ms, ss),
                        MiningState::Paused => assert_eq!(ms, is),
                        MiningState::Disabled => assert_eq!(ms, ss),
                        MiningState::Shutdown => assert_eq!(ms, ss),
                    }
                }
                Ok(())
            }

            pub(super) fn mixed_pause_unpause_types() -> anyhow::Result<()> {
                // for each state, we make a new state-machine and force it
                // to the target state, then pause and unpause it.
                let mut machine = MiningStateMachine::new(true, true);

                let mut paused_by_rpc = false;
                let mut paused_while_syncing = false;
                let mut paused_need_connection = false;

                let mut status = all_status();
                let mut events = all_pause_and_unpause_events()
                    .into_iter()
                    .flat_map(|(a, b)| [a, b])
                    .collect_vec();

                for _ in 0..50 {
                    status.shuffle(&mut rng());
                    events.shuffle(&mut rng());

                    // force to this random status.  (not allowed by API)
                    machine.status = status.iter().cloned().next().unwrap();

                    for event in events.iter() {
                        match *event {
                            MiningEvent::PauseNeedConnection => paused_need_connection = true,
                            MiningEvent::UnPauseNeedConnection => paused_need_connection = false,
                            MiningEvent::PauseRpc => paused_by_rpc = true,
                            MiningEvent::UnPauseRpc => paused_by_rpc = false,
                            MiningEvent::PauseSyncBlock => paused_while_syncing = true,
                            MiningEvent::UnPauseSyncBlock => paused_while_syncing = false,
                            _ => {}
                        }
                        machine.handle_event(*event)?;
                    }
                }

                // verify that machine pause flags match ours.
                assert_eq!(paused_by_rpc, machine.paused_by_rpc);
                assert_eq!(paused_while_syncing, machine.paused_while_syncing);
                assert_eq!(paused_need_connection, machine.paused_need_connection);

                let paused_count =
                    paused_by_rpc as u8 + paused_while_syncing as u8 + paused_need_connection as u8;
                assert_eq!(paused_count, machine.paused_count());

                Ok(())
            }

            fn all_status() -> Vec<MiningStatus> {
                let mut ms: Vec<_> = vec![];
                for state in MiningState::iter() {
                    if state == MiningState::Paused {
                        for reason in MiningPausedReason::iter() {
                            ms.push(MiningStatus::paused(reason))
                        }
                    } else {
                        ms.push(MiningStatus::try_from(state).unwrap());
                    }
                }
                ms
            }

            pub(super) fn can_pause_all_along_happy_path(
                pause_event: MiningEvent,
            ) -> anyhow::Result<()> {
                // for each status in happy path, we make a new state-machine and advance it
                // to the target state, then pause it.
                for status in compose_and_guess_happy_path() {
                    let mut machine = MiningStateMachine::new(true, true);
                    advance_init_to_status(&mut machine, status.state())?;
                    machine.handle_event(pause_event)?;
                }
                Ok(())
            }

            fn advance_init_to_status(
                machine: &mut MiningStateMachine,
                target: MiningState,
            ) -> anyhow::Result<()> {
                for status in compose_and_guess_happy_path() {
                    let state = status.state();
                    machine.advance_to(status)?;
                    if state == target {
                        break;
                    }
                }
                Ok(())
            }
        }
    }

    mod events {
        use super::*;

        #[test]
        fn compose_and_guess_happy_path() -> anyhow::Result<()> {
            let mut machine = MiningStateMachine::new(true, true);
            machine.exec_events(worker::compose_and_guess_happy_path())?;
            Ok(())
        }

        #[test]
        fn compose_happy_path() -> anyhow::Result<()> {
            let mut machine = MiningStateMachine::new(true, true);
            machine.exec_events(worker::compose_happy_path())?;
            Ok(())
        }

        #[test]
        fn guess_happy_path() -> anyhow::Result<()> {
            let mut machine = MiningStateMachine::new(true, true);
            machine.exec_events(worker::guess_happy_path())?;
            Ok(())
        }

        mod worker {
            use super::*;

            pub(super) fn compose_happy_path() -> Vec<MiningEvent> {
                vec![
                    MiningEvent::Advance, // Init        --> AwaitBlockProposal --> Composing
                    MiningEvent::Advance, // Composing   --> AwaitBlock
                    MiningEvent::Advance, // AwaitBlock  --> Guessing           --> NewTipBlock
                    MiningEvent::Advance, // NewTipBlock --> Init
                ]
            }

            pub(super) fn guess_happy_path() -> Vec<MiningEvent> {
                vec![
                    MiningEvent::Advance, // Init               --> AwaitBlockProposal
                    MiningEvent::Advance, // AwaitBlockProposal --> Composing          --> AwaitBlock --> Guessing
                    MiningEvent::Advance, // Guessing           --> NewTipBlock
                    MiningEvent::Advance, // NewTipBlock        --> Init
                ]
            }

            pub(super) fn compose_and_guess_happy_path() -> Vec<MiningEvent> {
                vec![
                    MiningEvent::Advance, // Init               --> AwaitBlockProposal  --> Composing
                    MiningEvent::Advance, // Composing          --> AwaitBlock          --> Guessing
                    MiningEvent::Advance, // Guessing           --> NewTipBlock
                    MiningEvent::Advance, // NewTipBlock        --> Init
                ]
            }
        }
    }
}
