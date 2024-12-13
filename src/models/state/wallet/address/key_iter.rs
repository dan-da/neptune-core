use serde::Deserialize;
use serde::Serialize;

use super::DerivationIndex;
use super::SpendingKey;

// an endless iterator over spending keys
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingKeyIter {
    parent_key: SpendingKey,
    curr: Option<DerivationIndex>,
}
impl SpendingKeyIter {
    pub fn new(parent_key: SpendingKey) -> Self {
        Self {
            parent_key,
            curr: Some(0),
        }
    }
}

impl Iterator for SpendingKeyIter {
    type Item = SpendingKey;

    fn next(&mut self) -> Option<Self::Item> {
        match self.curr {
            Some(curr) => {
                let key = self.parent_key.derive_child(curr);
                self.curr = Some(curr + 1);
                Some(key)
            }
            None => {
                self.curr = Some(1);
                Some(self.parent_key.derive_child(0))
            }
        }
    }

    // returns a tuple where the first element is the lower bound, and the
    // second element is the upper bound
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = usize::MAX;
        (len, Some(len))
    }
}

impl DoubleEndedIterator for SpendingKeyIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self.curr {
            Some(curr) if curr == 0 => {
                let key = self.parent_key.derive_child(curr);
                self.curr = None;
                Some(key)
            }
            Some(curr) if curr > 0 => {
                let key = self.parent_key.derive_child(curr);
                self.curr = Some(curr - 1);
                Some(key)
            }
            _ => None,
        }
    }
}

// note: Iterator::size_hint() must return exact size
impl ExactSizeIterator for SpendingKeyIter {}

// an iterator over a range of spending keys
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingKeyRangeIter {
    parent_key: SpendingKey,
    first: DerivationIndex,
    last: DerivationIndex,
    curr: Option<DerivationIndex>,
}

impl SpendingKeyRangeIter {
    pub fn new(parent_key: SpendingKey, first: DerivationIndex, last: DerivationIndex) -> Self {
        let curr = Some(first);
        Self {
            parent_key,
            first,
            last,
            curr,
        }
    }

    pub fn derive_nth(&self, index: DerivationIndex) -> SpendingKey {
        self.parent_key.derive_child(index)
    }
}
impl Iterator for SpendingKeyRangeIter {
    type Item = SpendingKey;

    fn next(&mut self) -> Option<Self::Item> {
        match self.curr {
            Some(curr) if curr <= self.last => {
                let key = self.parent_key.derive_child(curr);
                self.curr = Some(curr + 1);
                Some(key)
            }
            None => {
                self.curr = Some(self.first + 1);
                Some(self.parent_key.derive_child(self.first))
            }
            _ => None,
        }
    }

    // returns a tuple where the first element is the lower bound, and the
    // second element is the upper bound
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len: usize = (self.last - self.first) as usize;
        (len, Some(len))
    }
}

impl DoubleEndedIterator for SpendingKeyRangeIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self.curr {
            Some(curr) if curr == 0 => {
                let key = self.parent_key.derive_child(curr);
                self.curr = None;
                Some(key)
            }
            Some(curr) if curr >= self.first => {
                let key = self.parent_key.derive_child(curr);
                self.curr = Some(curr - 1);
                Some(key)
            }
            _ => None,
        }
    }
}

// note: Iterator::size_hint() must return exact size
impl ExactSizeIterator for SpendingKeyRangeIter {}

mod rayon {
    use super::*;
    use ::rayon::iter::plumbing::bridge;
    use ::rayon::iter::plumbing::Consumer;
    use ::rayon::iter::plumbing::Producer;
    use ::rayon::iter::plumbing::ProducerCallback;
    use ::rayon::iter::plumbing::UnindexedConsumer;
    use ::rayon::prelude::ParallelIterator;
    use ::rayon::prelude::IndexedParallelIterator;

    mod iter {
        use super::*;

        impl ParallelIterator for SpendingKeyIter {
            type Item = SpendingKey;

            fn drive_unindexed<C>(self, consumer: C) -> C::Result
            where
                C: UnindexedConsumer<Self::Item>,
            {
                bridge(self, consumer)
            }
        }

        impl IndexedParallelIterator for SpendingKeyIter {
            fn with_producer<CB: ProducerCallback<Self::Item>>(self, callback: CB) -> CB::Output {
                callback.callback(SpendingKeyProducer::from(self))
            }

            fn drive<C: Consumer<Self::Item>>(self, consumer: C) -> C::Result {
                bridge(self, consumer)
            }

            fn len(&self) -> usize {
                ExactSizeIterator::len(self)
            }
        }

        struct SpendingKeyProducer(SpendingKeyIter);

        impl Producer for SpendingKeyProducer {
            type Item = SpendingKey;
            type IntoIter = SpendingKeyIter;

            fn into_iter(self) -> Self::IntoIter {
                self.0
            }

            fn split_at(self, index: usize) -> (Self, Self) {
                let iter = self.0;

                let left = SpendingKeyIter {
                    parent_key: iter.parent_key,
                    curr: Some((index - 1) as DerivationIndex),
                };
                let right = SpendingKeyIter {
                    parent_key: iter.parent_key,
                    curr: Some(index as DerivationIndex),
                };
                (Self(left), Self(right))
            }
        }

        impl From<SpendingKeyIter> for SpendingKeyProducer {
            fn from(iter: SpendingKeyIter) -> Self {
                Self(iter)
            }
        }
    }

    mod range_iter {
        use super::*;

        impl ParallelIterator for SpendingKeyRangeIter {
            type Item = SpendingKey;

            fn drive_unindexed<C>(self, consumer: C) -> C::Result
            where
                C: UnindexedConsumer<Self::Item>,
            {
                bridge(self, consumer)
            }

            fn opt_len(&self) -> Option<usize> {
                Some(ExactSizeIterator::len(self))
            }
        }

        impl IndexedParallelIterator for SpendingKeyRangeIter {
            fn with_producer<CB: ProducerCallback<Self::Item>>(self, callback: CB) -> CB::Output {
                callback.callback(SpendingKeyRangeProducer::from(self))
            }

            fn drive<C: Consumer<Self::Item>>(self, consumer: C) -> C::Result {
                bridge(self, consumer)
            }

            fn len(&self) -> usize {
                ExactSizeIterator::len(self)
            }
        }

        struct SpendingKeyRangeProducer(SpendingKeyRangeIter);

        impl Producer for SpendingKeyRangeProducer {
            type Item = SpendingKey;
            type IntoIter = SpendingKeyRangeIter;

            fn into_iter(self) -> Self::IntoIter {
                self.0
            }

            fn split_at(self, index: usize) -> (Self, Self) {
                let range_iter = self.0;

                let left = SpendingKeyRangeIter::new(
                    range_iter.parent_key,
                    range_iter.first,
                    (index - 1) as DerivationIndex,
                );
                let right = SpendingKeyRangeIter::new(
                    range_iter.parent_key,
                    index as DerivationIndex,
                    range_iter.last,
                );
                (Self(left), Self(right))
            }
        }

        impl From<SpendingKeyRangeIter> for SpendingKeyRangeProducer {
            fn from(range_iter: SpendingKeyRangeIter) -> Self {
                Self(range_iter)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod iter {
        use super::*;
        use crate::models::state::wallet::symmetric_key::SymmetricKey;

        #[test]
        pub fn iterator() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            worker::iterator(parent_key, parent_key.into_iter());
        }

        #[test]
        pub fn range_iterator() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            worker::iterator(parent_key, parent_key.into_range_iter(0, 50));
        }

        #[test]
        pub fn double_ended_iterator() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            worker::double_ended_iterator(parent_key, parent_key.into_iter(), usize::MAX as DerivationIndex);
        }

        #[test]
        pub fn double_ended_range_iterator() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            let len = 50;
            worker::double_ended_iterator(parent_key, parent_key.into_range_iter(0, len), len);
        }

        mod worker {
            use super::*;

            pub fn iterator(parent_key: SpendingKey, mut iter: impl Iterator<Item=SpendingKey>) {
                for n in 0..5 {
                    assert_eq!(
                        Some(parent_key.derive_child(n)),
                        iter.next()
                    );
                }
            }

            pub fn double_ended_iterator(parent_key: SpendingKey, mut iter: impl DoubleEndedIterator<Item=SpendingKey>, len: DerivationIndex) {
                for n in 0..5 {
                    assert_eq!(
                        Some(parent_key.derive_child(n)),
                        iter.next()
                    );
                }
                for n in (len-5..len).rev() {
                    assert_eq!(
                        Some(parent_key.derive_child(n)),
                        iter.next_back()
                    );
                }
            }

        }
    }

    mod par_iter {

    }
}
