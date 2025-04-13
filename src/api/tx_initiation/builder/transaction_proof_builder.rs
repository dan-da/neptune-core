//! This module implements a builder for transaction proofs.
//!
//! There are different levels of [TransactionProof] that
//! can be generated.  The desired proof can be specified with [TransactionProofType].
//!
//! With exception of `TransactionProofType::PrimitiveWitness`, proof generation is a very CPU and RAM intensive process.  Each type
//! of proof has different hardware requirements.  Also the complexity is
//! affected by the type and size of transaction.
//!
//! It is necessary to inform the builder of the device's [TxProvingCapability]
//! so that weak devices will not attempt to build proofs they are not capable of.
//!
//! Before a transaction can be confirmed in a block it must have a SingleProof
//! which is the hardest proof to generate.
//!
//! see [Transaction Initiation Sequence](super::super#transaction-initiation-sequence)
//!
//! If you have a powerful enough machine, you can generate a ProofCollection or
//! SingleProof yourself before passing the transaction to neptune-core.  This
//! takes load off the entire network and may lower the transaction fee
//! requirements.
//!
//! see [Client Provides Proof Initiation Sequence](super::super#client-provides-proof-initiation-sequence)
//!
//! see [builder](super) for examples of using the builders together.

use std::borrow::Borrow;
use std::borrow::Cow;
use std::sync::Arc;

use crate::api::tx_initiation::error::CreateProofError;
use crate::job_queue::triton_vm::vm_job_queue;
use crate::job_queue::triton_vm::TritonVmJobQueue;
use crate::models::blockchain::transaction::primitive_witness::PrimitiveWitness;
use crate::models::blockchain::transaction::transaction_proof::TransactionProofType;
use crate::models::blockchain::transaction::validity::proof_collection::ProofCollection;
use crate::models::blockchain::transaction::validity::single_proof::SingleProof;
use crate::models::blockchain::transaction::TransactionProof;
use crate::models::proof_abstractions::tasm::program::TritonVmProofJobOptions;
use crate::models::state::transaction_details::TransactionDetails;

/// a builder for [TransactionProof]
///
/// see [module docs](self) for details.
#[derive(Debug, Default)]
pub struct TransactionProofBuilder<'a> {
    transaction_details: Option<&'a TransactionDetails>,
    primitive_witness: Option<PrimitiveWitness>,
    primitive_witness_ref: Option<&'a PrimitiveWitness>,
    job_queue: Option<Arc<TritonVmJobQueue>>,
    proof_job_options: Option<TritonVmProofJobOptions>,
    proof_type: Option<TransactionProofType>,
    valid_mock: Option<bool>,
}

impl<'a> TransactionProofBuilder<'a> {
    /// instantiate
    pub fn new() -> Self {
        Default::default()
    }

    /// add transaction details
    pub fn transaction_details(mut self, transaction_details: &'a TransactionDetails) -> Self {
        self.transaction_details = Some(transaction_details);
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

    /// specify the target proof type.  (optional)
    ///
    /// if not specified, then builder attempts to generate the
    /// best proof the device is capable of, as specified by
    /// tx_proving_capability().
    pub fn proof_type(mut self, proof_type: TransactionProofType) -> Self {
        self.proof_type = Some(proof_type);
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
    /// if the target proof-type is Witness, this will return immediately.
    ///
    /// if the network is [Network::RegTest], this will return immediately with
    /// a mock SingleProof.
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
    /// * proof_type(),
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
    pub async fn build(self) -> Result<TransactionProof, CreateProofError> {
        let TransactionProofBuilder {
            transaction_details,
            primitive_witness,
            primitive_witness_ref,
            job_queue,
            proof_job_options,
            valid_mock,
            proof_type,
        } = self;

        let Some(proof_job_options) = proof_job_options else {
            return Err(CreateProofError::MissingRequirement);
        };

        let capability = proof_job_options.job_settings.tx_proving_capability;
        let proof_type = proof_type.unwrap_or(capability.into());

        let valid_mock = valid_mock.unwrap_or(true);

        let job_queue = job_queue.unwrap_or_else(vm_job_queue);

        let build_inner = |witness_cow: Cow<'a, PrimitiveWitness>| async move {
            if proof_job_options.job_settings.network.use_mock_proof() {
                let proof = match proof_type {
                    TransactionProofType::PrimitiveWitness => {
                        TransactionProof::Witness(witness_cow.into_owned())
                    }
                    TransactionProofType::ProofCollection => {
                        let pc = ProofCollection::produce_mock(witness_cow.borrow(), valid_mock);
                        TransactionProof::ProofCollection(pc)
                    }
                    TransactionProofType::SingleProof => {
                        let sp = SingleProof::produce_mock(valid_mock);
                        TransactionProof::SingleProof(sp)
                    }
                };
                return Ok(proof);
            }

            if !capability.can_prove(proof_type) {
                return Err(CreateProofError::TooWeak {
                    proof_type,
                    capability,
                });
            }

            let transaction_proof = match proof_type {
                TransactionProofType::PrimitiveWitness => {
                    TransactionProof::Witness(witness_cow.into_owned())
                }
                TransactionProofType::ProofCollection => TransactionProof::ProofCollection(
                    ProofCollection::produce(witness_cow.borrow(), job_queue, proof_job_options)
                        .await?,
                ),
                TransactionProofType::SingleProof => TransactionProof::SingleProof(
                    SingleProof::produce(witness_cow.borrow(), job_queue, proof_job_options)
                        .await?,
                ),
            };

            Ok(transaction_proof)
        };

        match primitive_witness {
            Some(w) => build_inner(Cow::Owned(w)).await,
            None => match primitive_witness_ref {
                Some(w) => build_inner(Cow::Borrowed(w)).await,
                None => match transaction_details {
                    Some(d) => build_inner(Cow::Owned(d.primitive_witness())).await,
                    None => Err(CreateProofError::MissingRequirement),
                },
            },
        }
    }
}
