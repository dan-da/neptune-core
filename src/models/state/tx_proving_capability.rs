use std::fmt::Display;
use std::str::FromStr;

use clap::error::ErrorKind;
use serde::Deserialize;
use serde::Serialize;

use crate::models::blockchain::transaction::transaction_proof::TransactionProofType;
use crate::models::state::Claim;
use crate::models::state::NonDeterminism;
use crate::models::state::Program;
use crate::models::state::VMState;

// note: we should consider merging TransactionProofType and TxProvingCapability

/// represents which type of proof a given device is capable of generating
///
/// see also:
/// * [TransactionProofType]
/// * [TransactionProof](crate::models::blockchain::transaction::transaction_proof::TransactionProof)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TxProvingCapability {
    LockScript,
    #[default]
    PrimitiveWitness,
    ProofCollection,
    SingleProof,

    // In a real implementation, TxProvingCapability would be a struct, and this
    // would be a field, likely the only field.
    //
    // Here we make it a variant so we can make a quick prototype demonstrating
    // can_prove_claim_triple() without need to refactor all code that uses
    // TxProvingCapability.
    Log2PaddedHeight(u8),
}

impl From<TxProvingCapability> for TransactionProofType {
    fn from(c: TxProvingCapability) -> Self {
        match c {
            // TransactionProofType and TxProvingCapability need to be
            // reconciled with regards to LockScript, or merged into single type.
            TxProvingCapability::LockScript => unimplemented!(),
            TxProvingCapability::PrimitiveWitness => Self::PrimitiveWitness,
            TxProvingCapability::ProofCollection => Self::ProofCollection,
            TxProvingCapability::SingleProof => Self::SingleProof,
            TxProvingCapability::Log2PaddedHeight(max) if max >= 11 => Self::ProofCollection,
            TxProvingCapability::Log2PaddedHeight(max) if max >= 22 => Self::SingleProof,
            TxProvingCapability::Log2PaddedHeight(_) => Self::PrimitiveWitness,
        }
    }
}

impl TxProvingCapability {
    pub(crate) fn can_prove(&self, proof_type: TransactionProofType) -> bool {
        assert!(proof_type as u8 > 0);

        let self_val = match *self {
            // TransactionProofType and TxProvingCapability need to be
            // reconciled with regards to LockScript, or merged into single type.
            Self::LockScript => 0,
            Self::PrimitiveWitness => TransactionProofType::PrimitiveWitness as u8,
            Self::ProofCollection => TransactionProofType::ProofCollection as u8,
            Self::SingleProof => TransactionProofType::SingleProof as u8,
            Self::Log2PaddedHeight(max) if max >= 11 => TransactionProofType::ProofCollection as u8,
            Self::Log2PaddedHeight(max) if max >= 22 => TransactionProofType::SingleProof as u8,
            Self::Log2PaddedHeight(_) => TransactionProofType::PrimitiveWitness as u8,
        };

        self_val >= proof_type as u8
    }

    pub(crate) async fn can_prove_claim_triple(
        &self,
        program: Program,
        claim: Claim,
        nondeterminism: NonDeterminism,
    ) -> bool {
        let mut vmstate = VMState::new(program, claim.input.into(), nondeterminism);
        self.can_prove_claim(&mut vmstate)
    }

    pub(crate) async fn can_prove_claim(&self, vmstate: &mut VMState) -> bool {
        // note: this should be
        if let Self::Log2PaddedHeight(max) = *self {
            if vmstate.run().is_ok() {
                let big_max = 2u32.pow(max.into());
                return vmstate.cycle_count.next_power_of_two() < big_max;
            }
        }
        false
    }
}

impl Display for TxProvingCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                TxProvingCapability::PrimitiveWitness => "primitive witness",
                TxProvingCapability::LockScript => "lock script",
                TxProvingCapability::ProofCollection => "proof collection",
                TxProvingCapability::SingleProof => "single proof",
                TxProvingCapability::Log2PaddedHeight(_) => "log2-padded-height",
            }
        )
    }
}

impl FromStr for TxProvingCapability {
    type Err = clap::Error;
    // This implementation exists to allow CLI arguments to be converted to an
    // instance of this type.

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(value_str) = s.strip_prefix("Log2PaddedHeight=") {
            if let Ok(value) = value_str.parse::<u8>() {
                return Ok(TxProvingCapability::Log2PaddedHeight(value));
            } else {
                panic!("Invalid u8 value for Log2PaddedHeight");
                // return Err("Invalid u8 value for Log2PaddedHeight".to_string());
            }
        }
        match s {
            // PrimitiveWitness is not covered here, as it's only used
            // internally, and cannot be set on the client.
            "lockscript" => Ok(TxProvingCapability::LockScript),
            "proofcollection" => Ok(TxProvingCapability::ProofCollection),
            "singleproof" => Ok(TxProvingCapability::SingleProof),
            _ => Err(clap::Error::raw(
                ErrorKind::InvalidValue,
                "Invalid machine proving power",
            )),
        }
    }
}

use clap::ValueEnum;
use clap::builder::PossibleValue;

impl ValueEnum for TxProvingCapability {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            TxProvingCapability::LockScript,
            TxProvingCapability::PrimitiveWitness,
            TxProvingCapability::ProofCollection,
            TxProvingCapability::SingleProof,
            TxProvingCapability::Log2PaddedHeight(0), // Dummy value for variant listing
        ]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self {
            TxProvingCapability::LockScript => Some(PossibleValue::new("LockScript")),
            TxProvingCapability::PrimitiveWitness => Some(PossibleValue::new("PrimitiveWitness")),
            TxProvingCapability::ProofCollection => Some(PossibleValue::new("ProofCollection")),
            TxProvingCapability::SingleProof => Some(PossibleValue::new("SingleProof")),
            TxProvingCapability::Log2PaddedHeight(h) => {
                let s = format!("Log2PaddedHeight={}", h);
                let leaked_str: &'static str = Box::leak(s.into_boxed_str());
                Some(PossibleValue::new(leaked_str))
            }
        }
    }
}
