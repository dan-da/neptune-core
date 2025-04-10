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

/// defines Mock proof behaviors. (private)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, BFieldCodec)]
enum MockProofBehavior {
    ValidMock,
    InvalidMock,
}

/// represents a triton-vm proof that can optionally be mocked.
///
/// Mock proofs are useful for testing and simulations because they can be generated
/// instantly on commodity hardware whereas real proofs can take minutes on powerful
/// machines and simply be impossible to generate on weaker devices.
///
/// In particular the regtest network (mode) uses mock proofs so that transactions
/// and blocks can be generated quickly at will.
///
/// The proof can be of three types:
/// 1. standard.      not a mock proof
/// 2. valid-mock.    a mock proof that passes validation (if mock proofs are allowed)
/// 3. invalid-mock.  a mock proof that fails validation (if mock proofs are allowed, or not)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, GetSize, BFieldCodec, TasmObject)]
pub struct MockableProof {
    proof: VmProof,
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
    /// creates an invalid standard proof (not a mock proof)
    pub fn invalid() -> Self {
        Self {
            proof: VmProof(vec![]),
        }
    }

    /// creates a mock proof that will pass validation (if mock proofs are allowed)
    pub fn valid_mock(_claim: Claim) -> Self {
        Self {
            proof: VmProof(MockProofBehavior::ValidMock.encode()),
        }
    }

    /// creates a mock proof that will fail validation (if mock proofs are allowed, or not)
    pub fn invalid_mock(_claim: Claim) -> Self {
        Self {
            proof: VmProof(MockProofBehavior::InvalidMock.encode()),
        }
    }

    /// indicates if this is a standard proof (not a mock proof)
    pub fn is_standard(&self) -> bool {
        !self.is_valid_mock() && !self.is_invalid_mock()
    }

    /// indicates if this is a mock proof
    pub fn is_mock(&self) -> bool {
        self.is_valid_mock() || self.is_invalid_mock()
    }

    /// indicates if this is a valid mock proof
    pub fn is_valid_mock(&self) -> bool {
        self.matches_behavior(MockProofBehavior::ValidMock)
    }

    /// indicates if this is an invalid mock proof
    pub fn is_invalid_mock(&self) -> bool {
        self.matches_behavior(MockProofBehavior::InvalidMock)
    }

    fn matches_behavior(&self, target: MockProofBehavior) -> bool {
        if let Ok(behavior) = MockProofBehavior::decode(&self.proof.0) {
            *behavior == target
        } else {
            false
        }
    }
}

pub type Proof = MockableProof;
