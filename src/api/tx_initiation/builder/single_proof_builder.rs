//! This module implements a builder for transaction single-proofs.
//!
//! It is necessary to inform the builder of the device's [TxProvingCapability]
//! so that weak devices will not attempt to build proofs they are not capable of.
//!
//! Before a transaction can be confirmed in a block it must have a SingleProof
//! which is the hardest proof to generate.

use std::borrow::Borrow;
use std::borrow::Cow;
use std::sync::Arc;

use crate::api::tx_initiation::builder::proof_builder::ProofBuilder;
use crate::api::tx_initiation::error::CreateProofError;
use crate::job_queue::triton_vm::vm_job_queue;
use crate::job_queue::triton_vm::TritonVmJobQueue;
use crate::models::blockchain::transaction::primitive_witness::PrimitiveWitness;
use crate::models::blockchain::transaction::transaction_proof::TransactionProofType;
use crate::models::blockchain::transaction::validity::neptune_proof::Proof;
use crate::models::blockchain::transaction::validity::proof_collection::ProofCollection;
use crate::models::blockchain::transaction::validity::single_proof::SingleProof;
use crate::models::blockchain::transaction::validity::single_proof::SingleProofWitness;
use crate::models::proof_abstractions::tasm::program::ConsensusProgram;
use crate::models::proof_abstractions::tasm::program::TritonVmProofJobOptions;
use crate::models::proof_abstractions::SecretWitness;
use crate::models::state::transaction_details::TransactionDetails;
use crate::triton_vm::proof::Claim;
use crate::triton_vm::vm::NonDeterminism;

/// a builder for single proofs
///
/// see [module docs](self) for details.
#[derive(Debug, Default)]
pub struct SingleProofBuilder<'a> {
    transaction_details: Option<&'a TransactionDetails>,
    primitive_witness: Option<PrimitiveWitness>,
    primitive_witness_ref: Option<&'a PrimitiveWitness>,
    proof_collection: Option<ProofCollection>,
    single_proof_witness: Option<&'a SingleProofWitness>,
    claim_and_nondeterminism: Option<(Claim, NonDeterminism)>,
    job_queue: Option<Arc<TritonVmJobQueue>>,
    proof_job_options: Option<TritonVmProofJobOptions>,
    valid_mock: Option<bool>,
}

impl<'a> SingleProofBuilder<'a> {
    /// instantiate
    pub fn new() -> Self {
        Default::default()
    }

    /// add transaction details
    pub fn transaction_details(mut self, transaction_details: &'a TransactionDetails) -> Self {
        self.transaction_details = Some(transaction_details);
        self
    }

    /// add proof collection
    pub fn proof_collection(mut self, proof_collection: ProofCollection) -> Self {
        self.proof_collection = Some(proof_collection);
        self
    }

    /// add single proof witness
    pub fn single_proof_witness(mut self, single_proof_witness: &'a SingleProofWitness) -> Self {
        self.single_proof_witness = Some(single_proof_witness);
        self
    }

    /// add claim and non-determinism
    pub fn claim_and_nondeterminism(
        mut self,
        claim_and_nondeterminism: (Claim, NonDeterminism),
    ) -> Self {
        self.claim_and_nondeterminism = Some(claim_and_nondeterminism);
        self
    }

    /// add primitive witness
    ///
    /// If not provided, the builder will generate a `PrimitiveWitness` from the
    /// `TransactionDetails`.
    ///
    /// Note that a `PrimitiveWitness` contais a `TransactionKernel` which is
    /// an input to a `Transaction`.  Thus when generating a transaction
    /// it can avoid duplicate work to generate a witness, clone the kernel,
    /// provide the witness here, and then provide the kernel to the
    /// `TransactionBuilder`.
    ///
    /// It is also possible and may be more convenient to work only with
    /// `TransactionDetails`.
    pub fn primitive_witness(mut self, witness: PrimitiveWitness) -> Self {
        self.primitive_witness = Some(witness);
        self
    }

    /// add transaction details reference
    ///
    /// Note that if the target proof-type is `PrimitiveWitness` then the
    /// reference will be cloned when building and it may be better to use the
    /// `primitive_witness()` method.
    pub fn primitive_witness_ref(mut self, witness: &'a PrimitiveWitness) -> Self {
        self.primitive_witness_ref = Some(witness);
        self
    }

    /// add job queue (required)
    pub fn job_queue(mut self, job_queue: Arc<TritonVmJobQueue>) -> Self {
        self.job_queue = Some(job_queue);
        self
    }

    /// add job options. (optional)
    pub fn proof_job_options(mut self, proof_job_options: TritonVmProofJobOptions) -> Self {
        self.proof_job_options = Some(proof_job_options);
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

    /// generate the proof.
    ///
    /// if the network is [Network::RegTest], this will return immediately with
    /// a mock single proof.
    ///
    /// otherwise it will initiate an async job that could take many minutes.
    ///
    /// note that these jobs occur in a global (per process) job queue that only
    /// permits one VM job to process at a time.  This prevents parallel jobs
    /// from bringing the machine to its knees when each is using all available
    /// CPU cores and RAM.
    ///
    /// Given the serialized nature of the job-queue, it is possible or even likely
    /// that other jobs may precede this one.
    ///
    /// One can query the job_queue to determine how many jobs are in the queue.
    ///
    /// RegTest mode:
    ///
    /// mock proofs are used on the regtest network (only) because
    /// they can be generated instantly.
    ///
    /// When network is RegTest, these options are ignored by the builder:
    /// * proof_job_options()
    /// * job_queue()
    ///
    /// External Process:
    ///
    /// Proofs are generated in the Triton VM. The proof generation occurs in a
    /// separate executable, `triton-vm-prover`, which is spawned by the
    /// job-queue for each proving job.  Only one `triton-vm-prover` process
    /// should be executing at a time for a given neptune-core instance.
    ///
    /// If the external process is killed for any reason, the proof-generation job will fail
    /// and this method will return an error.
    ///
    /// Cancellation:
    ///
    /// note that cancelling the future returned by build() will NOT cancel the
    /// job in the job-queue, as that runs in a separately spawned tokio task
    /// managed by the job-queue.
    ///
    /// Although the job-queue provides a method for cancelling jobs, this builder
    /// does not presently expose it.  As such, there is no way to cancel a job
    /// once build() is called.  That funtionality may be exposed later.
    pub async fn build(self) -> Result<Proof, CreateProofError> {
        let SingleProofBuilder {
            transaction_details,
            primitive_witness,
            primitive_witness_ref,
            proof_collection,
            single_proof_witness,
            claim_and_nondeterminism,
            job_queue,
            proof_job_options,
            valid_mock,
        } = self;

        let Some(proof_job_options) = proof_job_options else {
            return Err(CreateProofError::MissingRequirement);
        };

        if proof_job_options.job_settings.network.use_mock_proof() {
            return Ok(Proof::mock(valid_mock.unwrap_or(true)));
        }

        let job_queue = job_queue.unwrap_or_else(vm_job_queue);

        let capability = proof_job_options.job_settings.tx_proving_capability;
        let proof_type = TransactionProofType::SingleProof;
        if !capability.can_prove(proof_type) {
            return Err(CreateProofError::TooWeak {
                proof_type,
                capability,
            });
        }

        let job_queue_clone = job_queue.clone();
        let proof_job_options_clone = proof_job_options.clone();

        let build_inner = |witness_cow: Cow<'a, PrimitiveWitness>| async move {
            Ok(SingleProof::produce(
                witness_cow.borrow(),
                job_queue_clone,
                proof_job_options_clone,
            )
            .await?)
        };

        match claim_and_nondeterminism {
            Some((claim, nondeterminism)) => {
                Self::prove_single_proof(claim, nondeterminism, job_queue, proof_job_options).await
            }
            _ => match single_proof_witness {
                Some(witness) => {
                    Self::prove_single_proof(
                        witness.claim(),
                        witness.nondeterminism(),
                        job_queue,
                        proof_job_options,
                    )
                    .await
                }
                _ => match proof_collection {
                    Some(pc) => {
                        let witness = SingleProofWitness::from_collection(pc);
                        Self::prove_single_proof(
                            witness.claim(),
                            witness.nondeterminism(),
                            job_queue,
                            proof_job_options,
                        )
                        .await
                    }
                    _ => match primitive_witness {
                        Some(w) => build_inner(Cow::Owned(w)).await,
                        None => match primitive_witness_ref {
                            Some(w) => build_inner(Cow::Borrowed(w)).await,
                            None => match transaction_details {
                                Some(d) => build_inner(Cow::Owned(d.primitive_witness())).await,
                                None => Err(CreateProofError::MissingRequirement),
                            },
                        },
                    },
                },
            },
        }
    }

    async fn prove_single_proof(
        claim: Claim,
        nondeterminism: NonDeterminism,
        job_queue: Arc<TritonVmJobQueue>,
        proof_job_options: TritonVmProofJobOptions,
    ) -> Result<Proof, CreateProofError> {
        ProofBuilder::new()
            .program(SingleProof.program())
            .claim(claim)
            .nondeterminism(nondeterminism)
            .job_queue(job_queue)
            .proof_job_options(proof_job_options)
            .build()
            .await
    }
}
