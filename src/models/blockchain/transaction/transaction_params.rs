//! This module implements TxParams which is used as input to
//! create_transaction() and the send() rpc.
use num_traits::Zero;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::models::blockchain::type_scripts::neptune_coins::NeptuneCoins;
use crate::models::consensus::timestamp::Timestamp;

use super::TxInputList;
use super::TxOutputList;

/// represents validation errors when constructing TxParams
#[derive(Debug, Clone, Error)]
pub enum TxParamsError {
    #[error("inputs ({inputs_sum}) is less than outputs ({outputs_sum})")]
    InsufficientInputs {
        inputs_sum: NeptuneCoins,
        outputs_sum: NeptuneCoins,
    },

    #[error("negative amount is not permitted for inputs or outputs")]
    NegativeAmount,

    #[error("zero amount is not permitted for inputs or outputs")]
    ZeroAmount,
}

// About serialization+validation
//
// the goal is to impl Deserialize with validation to ensure
// correct-by-construction using the "parse, don't validate" design philosophy.
//
// unfortunately serde does not yet directly support validating when using
// derive Deserialize.  So a workaround pattern is to create a shadow
// struct with the same fields that gets deserialized without validation
// and then use try_from to validate and construct the target.
//
// see: https://github.com/serde-rs/serde/issues/642#issuecomment-683276351

/// In RPC usage TxParams will be constructed first on the client, then
/// serialized via rpc, and deserialized on the server.
///
/// Basic validation of input/output amounts occurs when TxParams is constructed
/// including via deserialization.
///
/// This means that some validation occurs on the client as well as on the
/// server before create_transaction() is ever called.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "TxParamsShadow")]
pub struct TxParams {
    tx_input_list: TxInputList,
    tx_output_list: TxOutputList,
    timestamp: Timestamp,
}

// note: this only exists to get deserialized without validation.
#[derive(Deserialize)]
struct TxParamsShadow {
    tx_input_list: TxInputList,
    tx_output_list: TxOutputList,
    timestamp: Timestamp,
}

impl std::convert::TryFrom<TxParamsShadow> for TxParams {
    type Error = TxParamsError;

    fn try_from(s: TxParamsShadow) -> Result<Self, Self::Error> {
        Self::new_with_timestamp(s.tx_input_list, s.tx_output_list, s.timestamp)
    }
}

impl TxParams {
    /// construct a new TxParams using the current time
    pub fn new(tx_inputs: TxInputList, tx_outputs: TxOutputList) -> Result<Self, TxParamsError> {
        Self::new_with_timestamp(tx_inputs, tx_outputs, Timestamp::now())
    }

    /// construct a new TxParams with a custom timestamp
    pub fn new_with_timestamp(
        tx_input_list: TxInputList,
        tx_output_list: TxOutputList,
        timestamp: Timestamp,
    ) -> Result<Self, TxParamsError> {
        if tx_input_list.total_native_coins() < tx_output_list.total_native_coins() {
            return Err(TxParamsError::InsufficientInputs {
                inputs_sum: tx_input_list.total_native_coins(),
                outputs_sum: tx_output_list.total_native_coins(),
            });
        }

        // validate that all input and output amounts are non-negative and non-zero.
        for amount in tx_input_list
            .iter()
            .map(|i| i.utxo.get_native_currency_amount())
            .chain(
                tx_output_list
                    .iter()
                    .map(|o| o.utxo.get_native_currency_amount()),
            )
        {
            if amount.is_negative() {
                return Err(TxParamsError::NegativeAmount);
            }
            if amount.is_zero() {
                return Err(TxParamsError::ZeroAmount);
            }
        }

        Ok(Self {
            tx_input_list,
            tx_output_list,
            timestamp,
        })
    }

    /// return the fee amount which is sum(inputs) - sum(outputs)
    ///
    /// fee will always be >= 0, guaranteed by [Self::new()]
    pub fn fee(&self) -> NeptuneCoins {
        self.tx_input_list.total_native_coins() - self.tx_output_list.total_native_coins()
    }

    /// get the transaction inputs
    pub fn tx_input_list(&self) -> &TxInputList {
        &self.tx_input_list
    }

    /// get the transaction outputs
    pub fn tx_output_list(&self) -> &TxOutputList {
        &self.tx_output_list
    }

    /// get the timestamp
    pub fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }
}

// todo: tests for validation, serialization
