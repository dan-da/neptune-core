use super::super::super::traits::Job;
use super::super::super::traits::JobResult;

use crate::models::proof_abstractions::tasm::program::ConsensusProgramProver;
use crate::triton_vm::proof::Proof;

use std::any::Any;

#[derive(Debug)]
pub struct ConsensusProgramProverJobResult(pub Proof);
impl JobResult for ConsensusProgramProverJobResult {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl From<&ConsensusProgramProverJobResult> for Proof {
    fn from(v: &ConsensusProgramProverJobResult) -> Self {
        v.0.to_owned()
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
