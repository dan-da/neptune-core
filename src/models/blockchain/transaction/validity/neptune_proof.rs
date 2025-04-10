use std::ops::Deref;
use std::ops::DerefMut;

use get_size2::GetSize;
use serde::Deserialize;
use serde::Serialize;
use tasm_lib::prelude::Library;
use tasm_lib::structure::tasm_object::TasmObject;
use tasm_lib::triton_vm::proof::Claim;
use tasm_lib::triton_vm::proof::Proof as VmProof;

use crate::models::blockchain::transaction::BFieldCodec;
use crate::triton_vm::prelude::LabelledInstruction;
use crate::BFieldElement;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, BFieldCodec)]
enum MockableProofBehavior {
    ValidMock,
    InvalidMock,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, GetSize, BFieldCodec)]
pub struct MockableProof {
    proof: VmProof,
}

impl TasmObject for MockableProof {
    fn label_friendly_name() -> String {
        VmProof::label_friendly_name()
    }

    fn compute_size_and_assert_valid_size_indicator(
        library: &mut Library,
    ) -> Vec<LabelledInstruction> {
        VmProof::compute_size_and_assert_valid_size_indicator(library)
    }

    fn decode_iter<Itr: Iterator<Item = BFieldElement>>(
        iterator: &mut Itr,
    ) -> Result<Box<Self>, Box<dyn std::error::Error + Send + Sync>> {
        let elems: Vec<BFieldElement> = iterator.collect();
        let mockable_proof = Self::decode(&elems)?;
        Ok(mockable_proof)
    }
}

impl Deref for MockableProof {
    type Target = VmProof;

    fn deref(&self) -> &Self::Target {
        &self.proof
    }
}

impl DerefMut for MockableProof {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.proof
    }
}

impl From<MockableProof> for VmProof {
    fn from(mp: MockableProof) -> VmProof {
        mp.proof
    }
}

impl From<Vec<BFieldElement>> for MockableProof {
    fn from(v: Vec<BFieldElement>) -> Self {
        Self { proof: VmProof(v) }
    }
}

impl From<VmProof> for MockableProof {
    fn from(proof: VmProof) -> Self {
        Self { proof }
    }
}

impl MockableProof {
    pub fn invalid() -> Self {
        Self {
            proof: VmProof(vec![]),
        }
    }

    pub fn valid_mock(_claim: Claim) -> Self {
        Self {
            proof: VmProof(MockableProofBehavior::ValidMock.encode()),
        }
    }

    pub fn invalid_mock(_claim: Claim) -> Self {
        Self {
            proof: VmProof(MockableProofBehavior::InvalidMock.encode()),
        }
    }

    fn matches_behavior(&self, target: MockableProofBehavior) -> bool {
        if let Ok(behavior) = MockableProofBehavior::decode(&self.proof.0) {
            *behavior == target
        } else {
            false
        }
    }

    pub fn is_standard(&self) -> bool {
        !self.is_valid_mock() && !self.is_invalid_mock()
    }

    pub fn is_valid_mock(&self) -> bool {
        self.matches_behavior(MockableProofBehavior::ValidMock)
    }

    pub fn is_invalid_mock(&self) -> bool {
        self.matches_behavior(MockableProofBehavior::InvalidMock)
    }
}

pub type Proof = MockableProof;
