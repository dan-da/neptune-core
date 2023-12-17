use crate::util_types::sync::tokio as sync_tokio;

use crate::models::blockchain::block::block_header::BlockHeader;
use crate::models::blockchain::block::Block;

#[derive(Debug, Clone)]
pub struct LightState {
    // The documentation recommends using `std::sync::Mutex` for data that lives in memory,
    // but the `stad::sync::Mutex` cannot be held across `await` and that is too restrictive
    // at the moment, since we often want to hold multiple locks at the same time, and some
    // of these require calls to await.
    pub latest_block: sync_tokio::AtomicRw<Block>,
}

impl LightState {
    // TODO: Consider renaming to `new_threadsafe()` to reflect it does not return a `Self`.
    pub fn new(initial_latest_block: Block) -> Self {
        Self {
            latest_block: sync_tokio::AtomicRw::from(initial_latest_block),
        }
    }

    /// Locking:
    ///  * acquires read lock for `latest_block`
    pub async fn get_latest_block(&self) -> Block {
        self.latest_block.lock(|lb| lb.clone()).await
    }

    /// Locking:
    ///  * acquires read lock for `latest_block`
    pub async fn get_latest_block_header(&self) -> BlockHeader {
        self.latest_block.lock(|lb| lb.header.clone()).await
    }
}
