use crate::job_queue::Job;
use crate::job_queue::JobQueue;
use crate::main_loop::proof_upgrader::UpgradeJob;
use crate::models::blockchain::transaction::Transaction;
use crate::models::proof_abstractions::tasm::program::ConsensusProgramProver;
use crate::triton_vm::proof::Proof;

pub type VmJobQueue = JobQueue<TritonVmJob>;

/// primitive witness -> proof collection
/// primitive witness -> single proof
/// proof collection -> single proof
/// merge (of two single proofs)
/// update (updates mutator set data of one single proof)
#[derive(Debug)]
pub enum TritonVmJob {
    UpgradeProof {
        upgrade_job: UpgradeJob,
        triton_vm_job_queue: VmJobQueue,
    },
    ProveConsensusProgram(ConsensusProgramProver),
}

#[derive(Debug)]
pub enum TritonVmJobResult {
    UpgradeProof(Transaction),
    ProveConsensusProgram(Proof),
}

impl Job for TritonVmJob {
    type JobResult = anyhow::Result<TritonVmJobResult>;

    fn is_async(&self) -> bool {
        match self {
            Self::UpgradeProof { .. } => true,
            Self::ProveConsensusProgram(_) => false,
        }
    }

    async fn run_async(self) -> Self::JobResult {
        match self {
            Self::UpgradeProof {
                upgrade_job,
                triton_vm_job_queue,
            } => Ok(TritonVmJobResult::UpgradeProof(
                upgrade_job.upgrade(triton_vm_job_queue).await?,
            )),
            _ => unimplemented!(),
        }
    }

    fn run(self) -> Self::JobResult {
        match self {
            Self::ProveConsensusProgram(prover) => {
                Ok(TritonVmJobResult::ProveConsensusProgram(prover.prove()))
            }
            _ => unimplemented!(),
        }
    }
}
