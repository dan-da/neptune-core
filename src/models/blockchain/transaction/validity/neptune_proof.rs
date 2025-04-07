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


#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, GetSize, BFieldCodec)]
enum MockableProofBehavior {
    #[default]
    Standard,
    ValidMock,
    InvalidMock,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, GetSize)]
pub struct MockableProof {
    behavior: MockableProofBehavior,
    proof: VmProof,
}

impl BFieldCodec for MockableProof {
    type Error = <VmProof as BFieldCodec>::Error;

// this causes a runtime error when executing program in VM for some reason.

/*
    fn decode(sequence: &[BFieldElement]) -> Result<Box<Self>, Self::Error> {
        let behavior = *MockableProofBehavior::decode(&sequence[0..1]).unwrap(); // First element only
        let proof = *VmProof::decode(&sequence[1..])?; // All elements except the first

        Ok(Box::new(Self {
            behavior,
            proof,
        }))
    }

    fn encode(&self) -> Vec<BFieldElement> {
        let mut encoded = self.behavior.encode();
        encoded.extend(self.proof.encode());
        encoded
    }

    fn static_length() -> Option<usize> {
        Some(MockableProofBehavior::static_length()? + VmProof::static_length()?)
    } 
*/
    
    fn decode(sequence: &[BFieldElement]) -> Result<Box<Self>, Self::Error> {
        Ok(Box::new(Self{behavior: Default:: default(), proof: *VmProof::decode(sequence)?}))
    }


    fn encode(&self) -> Vec<BFieldElement> {
        self.proof.encode()
    }    

    fn static_length() -> Option<usize> {
        VmProof::static_length() 
    }

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
//        let proof = VmProof::decode_iter(iterator)?;
//        Ok(Box::new(Self{behavior: Default::default(), proof: *proof }))


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
        Self {
            proof: VmProof(v),
            behavior: Default::default(),
        }
    }
}

impl From<VmProof> for MockableProof {
    fn from(proof: VmProof) -> Self {
        Self {
            proof,
            behavior: Default::default(),
        }
    }
}

impl MockableProof {
    pub fn invalid() -> Self {
        Self {
            proof: VmProof(vec![]),
            behavior: MockableProofBehavior::Standard,
        }
    }

    pub fn valid_mock(_claim: Claim) -> Self {
        Self {
            proof: VmProof(vec![]),
            behavior: MockableProofBehavior::ValidMock,
        }
    }

    pub fn invalid_mock(_claim: Claim) -> Self {
        Self {
            proof: VmProof(vec![]),
            behavior: MockableProofBehavior::InvalidMock,
        }
    }

    pub fn is_standard(&self) -> bool {
        matches!(self.behavior, MockableProofBehavior::Standard)
    }

    pub fn is_valid_mock(&self) -> bool {
        matches!(self.behavior, MockableProofBehavior::ValidMock)
    }

    pub fn is_invalid_mock(&self) -> bool {
        matches!(self.behavior, MockableProofBehavior::InvalidMock)
    }
}

pub type Proof = MockableProof;
