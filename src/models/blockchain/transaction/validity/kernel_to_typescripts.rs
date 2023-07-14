use get_size::GetSize;
use serde::{Deserialize, Serialize};
use triton_vm::program::Program;

use super::{compiled_program::CompiledProgram, SupportedClaim, ValidationLogic};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, GetSize)]
pub struct KernelToTypeScripts {
    pub supported_claim: SupportedClaim,
}

impl KernelToTypeScripts {
    // TODO: Remove after implementing this struct
    pub fn dummy() -> Self {
        Self {
            supported_claim: SupportedClaim::dummy(),
        }
    }
}

impl ValidationLogic for KernelToTypeScripts {
    fn new_from_witness(
        _primitive_witness: &crate::models::blockchain::transaction::PrimitiveWitness,
        _tx_kernel: &crate::models::blockchain::transaction::transaction_kernel::TransactionKernel,
    ) -> Self {
        todo!()
    }

    fn prove(&mut self) -> anyhow::Result<()> {
        todo!()
    }

    fn verify(&self) -> bool {
        todo!()
    }
}

impl CompiledProgram for KernelToTypeScripts {
    fn rust_shadow(
        _public_input: std::collections::VecDeque<triton_vm::BFieldElement>,
        _secret_input: std::collections::VecDeque<triton_vm::BFieldElement>,
    ) -> Vec<triton_vm::BFieldElement> {
        todo!()
    }

    fn program() -> Program {
        todo!()
    }

    fn crash_conditions() -> Vec<String> {
        todo!()
    }
}
