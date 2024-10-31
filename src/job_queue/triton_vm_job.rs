use crate::job_queue::Job;
use crate::job_queue::JobQueue;
use crate::job_queue::JobResult;
use crate::models::proof_abstractions::tasm::program::ConsensusProgramProver;
use crate::triton_vm::proof::Proof;

use std::any::Any;

pub type VmJobQueue = JobQueue;

/// primitive witness -> proof collection
/// primitive witness -> single proof
/// proof collection -> single proof
/// merge (of two single proofs)
/// update (updates mutator set data of one single proof)

#[derive(Debug)]
pub struct ConsensusProgramProverJobResult(pub Proof);
impl JobResult for ConsensusProgramProverJobResult {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Clone)]
pub struct ConsensusProgramProverJob(pub ConsensusProgramProver);

#[async_trait::async_trait]
impl Job for ConsensusProgramProverJob {
    fn is_async(&self) -> bool {
        false
    }

    fn run(&self) -> Box<dyn JobResult> {
        let prover = self.0.to_owned();
        Box::new(ConsensusProgramProverJobResult(prover.prove()))
    }
}
