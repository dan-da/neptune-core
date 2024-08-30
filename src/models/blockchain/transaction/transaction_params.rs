use serde::Deserialize;
use serde::Serialize;

use crate::models::blockchain::type_scripts::neptune_coins::NeptuneCoins;
use crate::models::consensus::timestamp::Timestamp;

use super::TxInputList;
use super::TxOutputList;

// the goal is to impl Deserialize with validation to ensure
// correct-by-construction using the "parse, don't validate" design philosophy.
//
// unfortunately serde does not yet directly support validating when using
// derive Deserialize.  So a workaround pattern is to create a shadow
// struct with the same fields that gets deserialized without validation
// and then use try_from to validate and construct the target.
//
// see: https://github.com/serde-rs/serde/issues/642#issuecomment-683276351

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
    type Error = anyhow::Error;
    fn try_from(s: TxParamsShadow) -> Result<Self, Self::Error> {
        Self::new_with_timestamp(s.tx_input_list, s.tx_output_list, s.timestamp)
    }
}

impl TxParams {
    pub fn new(tx_inputs: TxInputList, tx_outputs: TxOutputList) -> anyhow::Result<Self> {
        Self::new_with_timestamp(tx_inputs, tx_outputs, Timestamp::now())
    }

    pub fn new_with_timestamp(
        tx_input_list: TxInputList,
        tx_output_list: TxOutputList,
        timestamp: Timestamp,
    ) -> anyhow::Result<Self> {
        if tx_input_list.total_native_coins() < tx_output_list.total_native_coins() {
            anyhow::bail!("outputs exceed inputs.");
        }

        Ok(Self {
            tx_input_list,
            tx_output_list,
            timestamp,
        })
    }

    /// fee will always be >= 0, guaranteed by Self::new()
    pub fn fee(&self) -> NeptuneCoins {
        self.tx_input_list.total_native_coins() - self.tx_output_list.total_native_coins()
    }

    pub fn tx_input_list(&self) -> &TxInputList {
        &self.tx_input_list
    }

    pub fn tx_output_list(&self) -> &TxOutputList {
        &self.tx_output_list
    }

    pub fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }
}
