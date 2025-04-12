//! This module implements a builder for neptune proofs
//!
//! A neptune proof is a mockable triton-vm proof.
use std::sync::Arc;

use crate::api::tx_initiation::error::CreateProofError;
use crate::job_queue::triton_vm::vm_job_queue;
use crate::job_queue::triton_vm::TritonVmJobQueue;
use crate::models::blockchain::transaction::transaction_proof::TransactionProofType;
use crate::models::blockchain::transaction::validity::neptune_proof::Proof;
use crate::models::proof_abstractions::tasm::program::prove_consensus_program;
use crate::models::proof_abstractions::tasm::program::TritonVmProofJobOptions;
use crate::models::state::tx_proving_capability::TxProvingCapability;
use crate::triton_vm::prelude::Program;
use crate::triton_vm::proof::Claim;
use crate::triton_vm::vm::NonDeterminism;

/// a builder for [Proof]
///
/// see [module docs](self) for details.
#[derive(Debug, Default)]
pub struct ProofBuilder {
    program: Option<Program>,
    claim: Option<Claim>,
    nondeterminism: Option<NonDeterminism>,
    job_queue: Option<Arc<TritonVmJobQueue>>,
    proof_job_options: TritonVmProofJobOptions,
    tx_proving_capability: Option<TxProvingCapability>,
    valid_mock: Option<bool>,
    suppress_capability_warning: bool,
}

impl ProofBuilder {
    /// instantiate
    pub fn new() -> Self {
        Default::default()
    }

    /// add program
    pub fn program(mut self, program: Program) -> Self {
        self.program = Some(program);
        self
    }

    /// add claim
    pub fn claim(mut self, claim: Claim) -> Self {
        self.claim = Some(claim);
        self
    }

    /// add nondeterminism
    pub fn nondeterminism(mut self, nondeterminism: NonDeterminism) -> Self {
        self.nondeterminism = Some(nondeterminism);
        self
    }

    /// add job queue (required)
    pub fn job_queue(mut self, job_queue: Arc<TritonVmJobQueue>) -> Self {
        self.job_queue = Some(job_queue);
        self
    }

    /// add job options. (optional)
    pub fn proof_job_options(mut self, proof_job_options: TritonVmProofJobOptions) -> Self {
        self.proof_job_options = proof_job_options;
        self
    }

    /// specify the device's proving capability.  (optional)
    pub fn tx_proving_capability(mut self, tx_proving_capability: TxProvingCapability) -> Self {
        self.tx_proving_capability = Some(tx_proving_capability);
        self
    }

    /// specify the device's proving capability.  (optional)
    pub fn tx_proving_capability_option(
        mut self,
        tx_proving_capability: Option<TxProvingCapability>,
    ) -> Self {
        self.tx_proving_capability = tx_proving_capability;
        self
    }

    /// suppress warning if proving capability is not supplied.
    pub fn suppress_capability_warning(mut self) -> Self {
        self.suppress_capability_warning = true;
        self
    }

    /// create valid or invalid mock proof. (optional)
    ///
    /// default = true
    ///
    /// only applies if the network uses mock proofs, eg regtest.
    ///
    /// does not apply to TransactionProof::PrimitiveWitness
    pub fn valid_mock(mut self, valid_mock: bool) -> Self {
        self.valid_mock = Some(valid_mock);
        self
    }

    pub async fn build(self) -> Result<Proof, CreateProofError> {
        let Self {
            program,
            claim,
            nondeterminism,
            job_queue,
            proof_job_options,
            tx_proving_capability,
            valid_mock,
            suppress_capability_warning,
        } = self;

        let (Some(program), Some(claim), Some(nondeterminism)) = (program, claim, nondeterminism)
        else {
            return Err(CreateProofError::MissingRequirement);
        };

        if proof_job_options.job_settings.network.use_mock_proof() {
            tracing::debug!("USE MOCK PROOF");
            let proof = Proof::mock(valid_mock.unwrap_or(true));
            return Ok(proof);
        }
        tracing::debug!("NOT IN USE MOCK PROOF");

        match tx_proving_capability {
            Some(capability) => {
                if !capability.can_prove(TransactionProofType::SingleProof) {
                    return Err(CreateProofError::TooWeak);
                }
            }
            None => {
                if !suppress_capability_warning {
                    tracing::warn!("tx_proving_capability not set. proving might fail.")
                }
            }
        }

        let job_queue = job_queue.unwrap_or_else(vm_job_queue);

        Ok(
            prove_consensus_program(program, claim, nondeterminism, job_queue, proof_job_options)
                .await?,
        )
    }
}
