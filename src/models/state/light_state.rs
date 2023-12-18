use crate::models::blockchain::block::AtomicBlock;

/// LightState is just a thread-safe Block.
/// (always representing the latest block)
pub type LightState = AtomicBlock;
