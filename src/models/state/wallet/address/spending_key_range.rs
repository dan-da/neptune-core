use rayon::prelude::IntoParallelIterator;
use serde::Deserialize;
use serde::Serialize;

use super::par_iter::SpendingKeyParallelIter;
use super::DerivationIndex;
use super::SpendingKey;
use super::SpendingKeyIter;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpendingKeyRange {
    pub parent_key: SpendingKey,
    pub first: DerivationIndex,
    pub last: DerivationIndex,
}
impl IntoIterator for SpendingKeyRange {
    type Item = SpendingKey;
    type IntoIter = SpendingKeyIter;

    fn into_iter(self) -> Self::IntoIter {
        self.parent_key.into_range_iter(0..=self.last)
    }
}
impl IntoParallelIterator for SpendingKeyRange {
    type Item = SpendingKey;
    type Iter = SpendingKeyParallelIter;

    fn into_par_iter(self) -> Self::Iter {
        self.parent_key.into_par_range_iter(0..=self.last)
    }
}
impl SpendingKeyRange {
    /// instantiate a new `SpendingKeyRange`
    pub fn new(parent_key: SpendingKey, first: DerivationIndex, last: DerivationIndex) -> Self {
        Self {
            parent_key,
            first,
            last,
        }
    }

    /// create a [SpendingKeyIter]
    pub fn iter(&self) -> SpendingKeyIter {
        self.parent_key.into_range_iter(self.first..=self.last)
    }

    /// create a [SpendingKeyParallelIter]
    pub fn par_iter(&self) -> SpendingKeyParallelIter {
        self.parent_key.into_par_range_iter(self.first..=self.last)
    }
}
