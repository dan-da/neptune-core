use serde::Deserialize;
use serde::Serialize;

use super::DerivationIndex;
use super::SpendingKey;

// an endless iterator over spending keys
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingKeyIter {
    key: SpendingKey,
    curr: Option<DerivationIndex>,
}
impl SpendingKeyIter {
    pub fn new(key: SpendingKey) -> Self {
        Self { key, curr: Some(0) }
    }
}

impl Iterator for SpendingKeyIter {
    type Item = SpendingKey;

    fn next(&mut self) -> Option<Self::Item> {
        match self.curr {
            Some(curr) if curr == 0 => {
                self.curr = Some(curr+1);
                Some(self.key)
            }
            Some(curr) => {
                let key = self.key.derive_child(curr-1);
                self.curr = Some(curr+1);
                Some(key)
            }
            None => None
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
                self.curr = None;
                Some(self.key)
            }
            Some(curr) => {
                let key = self.key.derive_child(curr-1);
                self.curr = Some(curr-1);
                Some(key)
            }
            None => None
        }
    }
}

// note: Iterator::size_hint() must return exact size
impl ExactSizeIterator for SpendingKeyIter {}


// an iterator over a range of spending keys
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingKeyRangeIter {
    master: SpendingKey,
    first: DerivationIndex,
    last: DerivationIndex,
    curr: DerivationIndex,
}

impl SpendingKeyRangeIter {
    pub fn new(master: SpendingKey, first: DerivationIndex, last: DerivationIndex) -> Self {
        let curr = first;
        Self {
            master,
            first,
            last,
            curr,
        }
    }

    pub fn nth(&self, index: DerivationIndex) -> SpendingKey {
        self.master.derive_child(index)
    }
}
impl Iterator for SpendingKeyRangeIter {
    type Item = SpendingKey;

    fn next(&mut self) -> Option<Self::Item> {
        if self.curr <= self.last {
            let key = self.master.derive_child(self.curr);
            self.curr += 1;
            Some(key)
        } else {
            None
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
        if self.curr >= self.first {
            let key = self.master.derive_child(self.curr);
            self.curr -= 1;
            Some(key)
        } else {
            None
        }
     }
}

// note: Iterator::size_hint() must return exact size
impl ExactSizeIterator for SpendingKeyRangeIter {}

mod rayon {
    use super::*;
    use ::rayon::iter::plumbing::Consumer;
    use ::rayon::iter::plumbing::ProducerCallback;
    use ::rayon::iter::plumbing::UnindexedConsumer;
    use ::rayon::iter::ParallelIterator;
    use ::rayon::iter::plumbing::bridge;
    use ::rayon::prelude::IndexedParallelIterator;
    use ::rayon::iter::plumbing::Producer;

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

        struct SpendingKeyProducer(
            SpendingKeyIter,
        );

        impl Producer for SpendingKeyProducer {
            type Item = SpendingKey;
            type IntoIter = SpendingKeyIter;

            fn into_iter(self) -> Self::IntoIter {
                self.0
            }

            fn split_at(self, index: usize) -> (Self, Self) {
                let iter = self.0;

                let left = SpendingKeyIter {
                    key: iter.key,
                    curr: Some((index - 1) as DerivationIndex),
                };
                let right = SpendingKeyIter {
                    key: iter.key,
                    curr: Some(index as DerivationIndex),
                };
                (Self(left), Self(right))
            }
        }

        impl From<SpendingKeyIter> for SpendingKeyProducer {
            fn from(iter: SpendingKeyIter) -> Self {
                Self (iter)
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

        struct SpendingKeyRangeProducer(
            SpendingKeyRangeIter,
        );

        impl Producer for SpendingKeyRangeProducer {
            type Item = SpendingKey;
            type IntoIter = SpendingKeyRangeIter;

            fn into_iter(self) -> Self::IntoIter {
                self.0
            }

            fn split_at(self, index: usize) -> (Self, Self) {
                let range_iter = self.0;

                let left = SpendingKeyRangeIter::new(
                    range_iter.master,
                    range_iter.first,
                    (index - 1) as DerivationIndex,
                );
                let right = SpendingKeyRangeIter::new(
                    range_iter.master,
                    index as DerivationIndex,
                    range_iter.last,
                );
                (Self(left), Self(right))
            }
        }

        impl From<SpendingKeyRangeIter> for SpendingKeyRangeProducer {
            fn from(range_iter: SpendingKeyRangeIter) -> Self {
                Self (range_iter)
            }
        }
    }
}
