use get_size::GetSize;
use itertools::Itertools;
use serde::Deserialize;
use serde::Serialize;
use tasm_lib::structure::tasm_object::TasmObject;
use tasm_lib::triton_vm;
use tasm_lib::triton_vm::prelude::*;
use tasm_lib::triton_vm::proof::Claim;
use tasm_lib::triton_vm::stark::Stark;
use tasm_lib::twenty_first::util_types::algebraic_hasher::AlgebraicHasher;
use tasm_lib::Digest;
use tokio::sync::TryLockError;
use tracing::debug;
use tracing::info;

use super::collect_type_scripts::CollectTypeScriptsWitness;
use super::kernel_to_outputs::KernelToOutputsWitness;
use super::removal_records_integrity::RemovalRecordsIntegrity;
use crate::models::blockchain::shared::Hash;
use crate::models::blockchain::transaction::primitive_witness::PrimitiveWitness;
use crate::models::blockchain::transaction::validity::collect_lock_scripts::CollectLockScripts;
use crate::models::blockchain::transaction::validity::collect_lock_scripts::CollectLockScriptsWitness;
use crate::models::blockchain::transaction::validity::collect_type_scripts::CollectTypeScripts;
use crate::models::blockchain::transaction::validity::kernel_to_outputs::KernelToOutputs;
use crate::models::blockchain::transaction::validity::removal_records_integrity::RemovalRecordsIntegrityWitness;
use crate::models::blockchain::transaction::BFieldCodec;
use crate::models::proof_abstractions::mast_hash::MastHash;
use crate::models::proof_abstractions::tasm::program::ConsensusProgram;
use crate::models::proof_abstractions::tasm::program::TritonVmJobQueue;
use crate::models::proof_abstractions::SecretWitness;
use crate::triton_vm::proof::Proof;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, GetSize, BFieldCodec, TasmObject)]
pub struct ProofCollection {
    pub removal_records_integrity: Proof,
    pub collect_lock_scripts: Proof,
    pub lock_scripts_halt: Vec<Proof>,
    pub kernel_to_outputs: Proof,
    pub collect_type_scripts: Proof,
    pub type_scripts_halt: Vec<Proof>,
    pub lock_script_hashes: Vec<Digest>,
    pub type_script_hashes: Vec<Digest>,
    pub kernel_mast_hash: Digest,
    pub salted_inputs_hash: Digest,
    pub salted_outputs_hash: Digest,
}

impl ProofCollection {
    fn extract_specific_witnesses(
        primitive_witness: &PrimitiveWitness,
    ) -> (
        RemovalRecordsIntegrityWitness,
        CollectLockScriptsWitness,
        KernelToOutputsWitness,
        CollectTypeScriptsWitness,
    ) {
        // collect witnesses
        let removal_records_integrity_witness =
            RemovalRecordsIntegrityWitness::from(primitive_witness);
        let collect_lock_scripts_witness = CollectLockScriptsWitness::from(primitive_witness);
        let kernel_to_outputs_witness = KernelToOutputsWitness::from(primitive_witness);
        let collect_type_scripts_witness = CollectTypeScriptsWitness::from(primitive_witness);

        (
            removal_records_integrity_witness,
            collect_lock_scripts_witness,
            kernel_to_outputs_witness,
            collect_type_scripts_witness,
        )
    }
    pub fn can_produce(primitive_witness: &PrimitiveWitness) -> bool {
        let (
            removal_records_integrity_witness,
            collect_lock_scripts_witness,
            kernel_to_outputs_witness,
            collect_type_scripts_witness,
        ) = Self::extract_specific_witnesses(primitive_witness);

        // verify graceful halts
        let removal_records_integrity_halts = if let Ok(output) = RemovalRecordsIntegrity.run_rust(
            &removal_records_integrity_witness.standard_input(),
            removal_records_integrity_witness.nondeterminism(),
        ) {
            output == removal_records_integrity_witness.output()
        } else {
            false
        };

        let collect_lock_scripts_halts = if let Ok(output) = CollectLockScripts.run_rust(
            &collect_lock_scripts_witness.standard_input(),
            collect_lock_scripts_witness.nondeterminism(),
        ) {
            output == collect_lock_scripts_witness.output()
        } else {
            false
        };

        let kernel_to_outputs_halts = if let Ok(output) = KernelToOutputs.run_rust(
            &kernel_to_outputs_witness.standard_input(),
            kernel_to_outputs_witness.nondeterminism(),
        ) {
            output == kernel_to_outputs_witness.output()
        } else {
            false
        };

        let collect_type_scripts_halts = if let Ok(output) = CollectTypeScripts.run_rust(
            &collect_type_scripts_witness.standard_input(),
            collect_type_scripts_witness.nondeterminism(),
        ) {
            output == collect_type_scripts_witness.output()
        } else {
            false
        };

        let txk_mast_hash = primitive_witness.kernel.mast_hash();
        let txk_mast_hash_as_input = PublicInput::new(txk_mast_hash.reversed().values().to_vec());
        let salted_inputs_hash = Hash::hash(&primitive_witness.input_utxos);
        let salted_outputs_hash = Hash::hash(&primitive_witness.output_utxos);

        let all_lock_scripts_halt = primitive_witness
            .lock_scripts_and_witnesses
            .iter()
            .all(|lsaw| lsaw.halts_gracefully(txk_mast_hash_as_input.clone()));
        let all_type_scripts_halt = primitive_witness
            .type_scripts_and_witnesses
            .iter()
            .all(|ts| ts.halts_gracefully(txk_mast_hash, salted_inputs_hash, salted_outputs_hash));

        removal_records_integrity_halts
            || collect_lock_scripts_halts
            || kernel_to_outputs_halts
            || collect_type_scripts_halts
            || all_lock_scripts_halt
            || all_type_scripts_halt
    }

    pub(crate) async fn produce(
        primitive_witness: &PrimitiveWitness,
        sync_device: &TritonVmJobQueue,
    ) -> Result<Self, TryLockError> {
        let (
            removal_records_integrity_witness,
            collect_lock_scripts_witness,
            kernel_to_outputs_witness,
            collect_type_scripts_witness,
        ) = Self::extract_specific_witnesses(primitive_witness);

        let txk_mast_hash = primitive_witness.kernel.mast_hash();
        let txk_mast_hash_as_input = PublicInput::new(txk_mast_hash.reversed().values().to_vec());
        let salted_inputs_hash = Hash::hash(&primitive_witness.input_utxos);
        let salted_outputs_hash = Hash::hash(&primitive_witness.output_utxos);
        debug!("proving, txk hash: {}", txk_mast_hash);
        debug!("proving, salted inputs hash: {}", salted_inputs_hash);
        debug!("proving, salted outputs hash: {}", salted_outputs_hash);

        // prove
        debug!("proving RemovalRecordsIntegrity");
        let removal_records_integrity = RemovalRecordsIntegrity
            .prove(
                &removal_records_integrity_witness.claim(),
                removal_records_integrity_witness.nondeterminism(),
                sync_device,
            )
            .await?;

        debug!("proving CollectLockScripts");
        let collect_lock_scripts = CollectLockScripts
            .prove(
                &collect_lock_scripts_witness.claim(),
                collect_lock_scripts_witness.nondeterminism(),
                sync_device,
            )
            .await?;

        debug!("proving KernelToOutputs");
        let kernel_to_outputs = KernelToOutputs
            .prove(
                &kernel_to_outputs_witness.claim(),
                kernel_to_outputs_witness.nondeterminism(),
                sync_device,
            )
            .await?;

        debug!("proving CollectTypeScripts");
        let collect_type_scripts = CollectTypeScripts
            .prove(
                &collect_type_scripts_witness.claim(),
                collect_type_scripts_witness.nondeterminism(),
                sync_device,
            )
            .await?;

        debug!("proving lock scripts");
        let mut lock_scripts_halt = vec![];
        for lock_script_and_witness in primitive_witness.lock_scripts_and_witnesses.iter() {
            lock_scripts_halt.push(
                lock_script_and_witness
                    .prove(txk_mast_hash_as_input.clone(), sync_device)
                    .await?,
            );
        }

        debug!("proving type scripts");
        let mut type_scripts_halt = vec![];
        for (i, tsaw) in primitive_witness
            .type_scripts_and_witnesses
            .iter()
            .enumerate()
        {
            debug!("proving type script number {i}: {}", tsaw.program.hash());
            type_scripts_halt.push(
                tsaw.prove(
                    txk_mast_hash,
                    salted_inputs_hash,
                    salted_outputs_hash,
                    sync_device,
                )
                .await?,
            );
        }
        info!("done proving proof collection");

        // collect hashes
        let lock_script_hashes = primitive_witness
            .lock_scripts_and_witnesses
            .iter()
            .map(|lsaw| lsaw.program.hash())
            .collect_vec();
        let type_script_hashes = primitive_witness
            .type_scripts_and_witnesses
            .iter()
            .map(|tsaw| tsaw.program.hash())
            .collect_vec();

        // assemble data into struct and return
        Ok(ProofCollection {
            removal_records_integrity,
            collect_lock_scripts,
            lock_scripts_halt,
            kernel_to_outputs,
            collect_type_scripts,
            type_scripts_halt,
            lock_script_hashes,
            type_script_hashes,
            kernel_mast_hash: txk_mast_hash,
            salted_inputs_hash,
            salted_outputs_hash,
        })
    }

    pub fn verify(&self, txk_mast_hash: Digest) -> bool {
        debug!("verifying, txk hash: {}", txk_mast_hash);
        debug!("verifying, salted inputs hash: {}", self.salted_inputs_hash);
        debug!(
            "verifying, salted outputs hash: {}",
            self.salted_outputs_hash
        );
        // make sure we are talking about the same tx
        if self.kernel_mast_hash != txk_mast_hash {
            return false;
        }

        // compile claims
        let removal_records_integrity_claim = Claim {
            program_digest: RemovalRecordsIntegrity.program().hash(),
            input: self.kernel_mast_hash.reversed().values().to_vec(),
            output: self.salted_inputs_hash.values().to_vec(),
        };
        debug!(
            "removal records integrity claim: {:?}",
            removal_records_integrity_claim
        );
        let kernel_to_outputs_claim = Claim {
            program_digest: KernelToOutputs.program().hash(),
            input: self.kernel_mast_hash.reversed().values().to_vec(),
            output: self.salted_outputs_hash.values().to_vec(),
        };
        let collect_lock_scripts_claim = Claim {
            program_digest: CollectLockScripts.program().hash(),
            input: self.salted_inputs_hash.reversed().values().to_vec(),
            output: self
                .lock_script_hashes
                .iter()
                .flat_map(|d| d.values())
                .collect_vec(),
        };
        let collect_type_scripts_claim = Claim {
            program_digest: CollectTypeScripts.program().hash(),
            input: [self.salted_inputs_hash, self.salted_outputs_hash]
                .into_iter()
                .flat_map(|d| d.reversed().values())
                .collect_vec(),
            output: self
                .type_script_hashes
                .iter()
                .flat_map(|d| d.values())
                .collect_vec(),
        };
        let lock_script_claims = self
            .lock_script_hashes
            .iter()
            .map(|lsh| Claim {
                program_digest: *lsh,
                input: self.kernel_mast_hash.reversed().values().to_vec(),
                output: vec![],
            })
            .collect_vec();
        let type_script_claims = self
            .type_script_hashes
            .iter()
            .map(|tsh| {
                Claim::new(*tsh).with_input(
                    [
                        self.kernel_mast_hash,
                        self.salted_inputs_hash,
                        self.salted_outputs_hash,
                    ]
                    .into_iter()
                    .flat_map(|d| d.reversed().values())
                    .collect_vec(),
                )
            })
            .collect_vec();

        // verify
        debug!("verifying removal records integrity ...");
        let rri = triton_vm::verify(
            Stark::default(),
            &removal_records_integrity_claim,
            &self.removal_records_integrity,
        );
        debug!("{rri}");
        debug!("verifying kernel to outputs ...");
        let k2o = triton_vm::verify(
            Stark::default(),
            &kernel_to_outputs_claim,
            &self.kernel_to_outputs,
        );
        debug!("{k2o}");
        debug!("verifying collect lock scripts ...");
        let cls = triton_vm::verify(
            Stark::default(),
            &collect_lock_scripts_claim,
            &self.collect_lock_scripts,
        );
        debug!("{cls}");
        debug!("verifying collect type scripts ...");
        let cts = triton_vm::verify(
            Stark::default(),
            &collect_type_scripts_claim,
            &self.collect_type_scripts,
        );
        debug!("{cts}");
        debug!("verifying that all lock scripts halt ...");
        let lsh = lock_script_claims
            .iter()
            .zip(self.lock_scripts_halt.iter())
            .all(|(cl, pr)| triton_vm::verify(Stark::default(), cl, pr));
        debug!("{lsh}");
        debug!("verifying that all type scripts halt ...");
        let tsh = type_script_claims
            .iter()
            .zip(self.type_scripts_halt.iter())
            .all(|(cl, pr)| triton_vm::verify(Stark::default(), cl, pr));
        debug!("{tsh}");

        // and all bits together and return
        rri && k2o && cls && cts && lsh && tsh
    }

    pub fn removal_records_integrity_claim(&self) -> Claim {
        Claim {
            program_digest: RemovalRecordsIntegrity.program().hash(),
            input: self.kernel_mast_hash.reversed().values().to_vec(),
            output: self.salted_inputs_hash.values().to_vec(),
        }
    }

    pub fn kernel_to_outputs_claim(&self) -> Claim {
        Claim {
            program_digest: KernelToOutputs.program().hash(),
            input: self.kernel_mast_hash.reversed().values().to_vec(),
            output: self.salted_outputs_hash.values().to_vec(),
        }
    }

    pub fn collect_lock_scripts_claim(&self) -> Claim {
        let mut lock_script_hashes_as_output = vec![];
        let mut i: usize = 0;
        while i < self.lock_script_hashes.len() {
            let lock_script_hash: Digest = self.lock_script_hashes[i];
            let mut j: usize = 0;
            while j < Digest::LEN {
                lock_script_hashes_as_output.push(lock_script_hash.values()[j]);
                j += 1;
            }
            i += 1;
        }
        Claim {
            program_digest: CollectLockScripts.program().hash(),
            input: self.salted_inputs_hash.reversed().values().to_vec(),
            output: lock_script_hashes_as_output,
        }
    }

    pub fn collect_type_scripts_claim(&self) -> Claim {
        let mut type_script_hashes_as_output = vec![];
        let mut i = 0;
        while i < self.type_script_hashes.len() {
            let type_script_hash: Digest = self.type_script_hashes[i];
            let mut j: usize = 0;
            while j < Digest::LEN {
                type_script_hashes_as_output.push(type_script_hash.values()[j]);
                j += 1;
            }
            i += 1;
        }
        Claim {
            program_digest: CollectTypeScripts.program().hash(),
            input: [
                self.salted_inputs_hash.reversed().values().to_vec(),
                self.salted_outputs_hash.reversed().values().to_vec(),
            ]
            .concat(),
            output: type_script_hashes_as_output,
        }
    }

    pub fn lock_script_claims(&self) -> Vec<Claim> {
        let mut claims = vec![];
        let mut i = 0;
        while i < self.lock_script_hashes.len() {
            let lock_script_hash = self.lock_script_hashes[i];
            claims.push(Claim {
                program_digest: lock_script_hash,
                input: self.kernel_mast_hash.reversed().values().to_vec(),
                output: vec![],
            });

            i += 1;
        }

        claims
    }

    pub fn type_script_claims(&self) -> Vec<Claim> {
        let type_script_input = [
            self.kernel_mast_hash.reversed().values(),
            self.salted_inputs_hash.reversed().values(),
            self.salted_outputs_hash.reversed().values(),
        ]
        .concat();
        let mut claims = vec![];
        let mut i = 0;
        while i < self.type_script_hashes.len() {
            let type_script_hash = self.type_script_hashes[i];
            claims.push(Claim {
                program_digest: type_script_hash,
                input: type_script_input.clone(),
                output: vec![],
            });
            i += 1;
        }
        claims
    }
}

#[cfg(test)]
pub mod test {
    use proptest::prop_assert;
    use test_strategy::proptest;

    use super::*;

    #[proptest(cases = 5)]
    fn can_produce_valid_collection(
        #[strategy(PrimitiveWitness::arbitrary_with((2, 2, 2)))]
        primitive_witness: PrimitiveWitness,
    ) {
        prop_assert!(ProofCollection::can_produce(&primitive_witness));
    }
}
