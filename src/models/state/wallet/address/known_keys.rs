use rayon::prelude::IntoParallelIterator;
use serde::Deserialize;
use serde::Serialize;

use super::par_iter::SpendingKeyParallelIter;
use super::DerivationIndex;
use super::SpendingKey;
use super::SpendingKeyIter;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnownSpendingKeys {
    pub parent_key: SpendingKey,
    pub last_known: DerivationIndex,
}
impl IntoIterator for KnownSpendingKeys {
    type Item = SpendingKey;
    type IntoIter = SpendingKeyIter;

    fn into_iter(self) -> Self::IntoIter {
        self.parent_key.into_range_iter(0..=self.last_known)
    }
}
impl IntoParallelIterator for KnownSpendingKeys {
    type Item = SpendingKey;
    type Iter = SpendingKeyParallelIter;

    fn into_par_iter(self) -> Self::Iter {
        self.parent_key.into_par_range_iter(0..=self.last_known)
    }
}
impl KnownSpendingKeys {

    /// instantiate a new `KnownSpendingKeys`
    pub fn new(parent_key: SpendingKey, last_known: DerivationIndex) -> Self {
        Self {
            parent_key,
            last_known,
        }
    }

    /// create a [SpendingKeyIter]
    pub fn iter(&self) -> SpendingKeyIter {
        self.parent_key.into_range_iter(0..=self.last_known)
    }

    /// create a [SpendingKeyParallelIter]
    pub fn par_iter(&self) -> SpendingKeyParallelIter {
        self.parent_key.into_par_range_iter(0..=self.last_known)
    }
}
