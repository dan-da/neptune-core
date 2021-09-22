use crate::model::{Block, FromMinerToMain, ToMiner, Transaction, Utxo};
use anyhow::Result;
use std::time::SystemTime;
use tokio::select;
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep, Duration};
use tracing::{info, instrument};

const MOCK_REGTEST_MINIMUM_MINE_INTERVAL_SECONDS: u64 = 4;

fn make_mock_block(height: u64) -> Block {
    let utxo_pol = [0u32; 2048];
    let utxo = Utxo {
        pol0: utxo_pol,
        pol1: utxo_pol,
    };

    let tx = Transaction {
        input: vec![utxo.clone()],
        output: vec![utxo.clone()],
        public_scripts: vec![],
        proof: vec![],
    };
    Block {
        version_bits: [0u8; 4],
        timestamp: SystemTime::now(),
        height,
        nonce: [0u8; 32],
        predecessor: [0u8; 32],
        predecessor_proof: vec![],
        accumulated_pow_line: 0u128,
        accumulated_pow_family: 0u128,
        uncles: vec![],
        target_difficulty: 0u128,
        retarget_proof: vec![],
        transaction: tx,
        mixed_edges: vec![],
        mix_proof: vec![],
        edge_mmra: utxo,
        edge_mmra_update: vec![],
        hash: [0u8; 32],
    }
}

#[instrument]
pub async fn mock_regtest_mine(
    mut from_main: watch::Receiver<ToMiner>,
    to_main: mpsc::Sender<FromMinerToMain>,
) -> Result<()> {
    let mut block_height = 0u64;
    loop {
        let rand_time: u64 = rand::random::<u64>() % 10;
        select! {
            _ = from_main.changed() => {
                let main_message: ToMiner = from_main.borrow().clone();
                match main_message {
                    ToMiner::NewBlock(block) => {
                        if block.height > block_height {
                            block_height = block.height;
                            info!("Miner thread received regtest block height {}", block_height);
                        }
                    }
                    ToMiner::Empty => ()
                }
            }
            _ = sleep(Duration::from_secs(MOCK_REGTEST_MINIMUM_MINE_INTERVAL_SECONDS + rand_time)) => {
                block_height += 1;

                to_main.send(FromMinerToMain::NewBlock(Box::new(make_mock_block(block_height)))).await?;
                info!("Found new regtest block with block height {}", block_height);
            }
        }
    }
}
