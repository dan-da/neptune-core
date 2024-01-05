use super::{archival_state::ArchivalState, light_state::LightState};

// silence possible clippy bug / false positive.
// see: https://github.com/rust-lang/rust-clippy/issues/9798
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum BlockchainState {
    Archival(BlockchainArchivalState),
    Light(LightState),
}

impl BlockchainState {
    #[inline]
    pub fn is_archival_node(&self) -> bool {
        matches!(self, Self::Archival(_))
    }

    #[inline]
    pub fn archival_state(&self) -> &ArchivalState {
        match self {
            Self::Archival(bac) => &bac.archival_state,
            Self::Light(_) => panic!("archival_state not available in LightState mode"),
        }
    }

    #[inline]
    pub fn archival_state_mut(&mut self) -> &mut ArchivalState {
        match self {
            Self::Archival(bac) => &mut bac.archival_state,
            Self::Light(_) => panic!("archival_state not available in LightState mode"),
        }
    }

    #[inline]
    pub fn light_state(&self) -> &LightState {
        match self {
            Self::Archival(bac) => &bac.light_state,
            Self::Light(light_state) => light_state,
        }
    }

    #[inline]
    pub fn light_state_mut(&mut self) -> &mut LightState {
        match self {
            Self::Archival(bac) => &mut bac.light_state,
            Self::Light(light_state) => light_state,
        }
    }
}

/// The `BlockchainState` contains database access to block headers.
///
/// It is divided into `ArchivalState` and `LightState`.
#[derive(Debug)]
pub struct BlockchainArchivalState {
    /// The `archival_state` locks require an await to be taken, so archival_state
    /// locks must always be taken before light state locks. Due to the policy of
    /// taking locks in the order they are defined in terms of fields, archival_state
    /// must be listed before `light_state`.
    pub archival_state: ArchivalState,

    /// The `LightState` contains a lock from std::sync which may not be held
    /// across an await.
    pub light_state: LightState,
}
