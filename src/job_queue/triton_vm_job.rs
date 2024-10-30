use crate::job_queue::Job;
use crate::job_queue::JobQueue;
use crate::main_loop::proof_upgrader::UpgradeJob;
use crate::models::blockchain::transaction::Transaction;

pub type VmJobQueue = JobQueue<TritonVmJob>;

/// primitive witness -> proof collection
/// primitive witness -> single proof
/// proof collection -> single proof
/// merge (of two single proofs)
/// update (updates mutator set data of one single proof)
#[derive(Debug, Clone)]
pub enum TritonVmJob {
    UpgradeProof(UpgradeJob)
}

impl Job for TritonVmJob {

    type JobResult = anyhow::Result<Transaction>;

    fn is_async(&self) -> bool {
        true
    }

    async fn run_async(self) -> Self::JobResult {
        match self {
            Self::UpgradeProof(upgrade_job) => Ok(upgrade_job.upgrade().await?),
        }
    }

    fn run(self) -> Self::JobResult {
        unimplemented!()
    }
}