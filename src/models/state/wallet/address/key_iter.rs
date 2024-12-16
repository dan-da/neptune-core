use serde::Deserialize;
use serde::Serialize;

use super::DerivationIndex;
use super::SpendingKey;

/// an iterator over a range of spending keys
///
/// Given a parent-key, performs iteration over any range of child keys
/// where:
///  1. The maximum range size is usize::MAX
///  2. The lowest reachable index is 0
///  3. the highest reachable index is DerivationIndex::MAX
///
/// Typically this iterator would be used for iterating over range
/// 0..max, where max is defined by the application, keeping in mind
/// that each iteration requires deriving a new child key, which is
/// a relatively expensive operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingKeyIter {
    parent_key: SpendingKey,
    range: std::ops::Range<DerivationIndex>,
    curr: Option<DerivationIndex>,
    curr_back: Option<DerivationIndex>,
}

impl From<SpendingKey> for SpendingKeyIter {
    fn from(parent_key: SpendingKey) -> Self {
        // max range size is usize::MAX because Iterator methods such as
        // size_hint() and len() use usize.
        Self::new_range(parent_key, 0..usize::MAX as DerivationIndex)
    }
}

impl SpendingKeyIter {
    /// creates a new iterator over child-keys of parent-key from
    /// derivation-index first to last.
    ///
    /// panics if any of these conditions are not true:
    ///   range.end >= range.start
    ///   range.end > 0
    ///   range.end - range.start <= usize::MAX
    pub fn new_range(parent_key: SpendingKey, range: std::ops::Range<DerivationIndex>) -> Self {
        assert!(range.end >= range.start);
        assert!(range.end > 0);
        assert!(range.end - range.start <= usize::MAX as DerivationIndex);

        Self {
            parent_key,
            curr: Some(range.start),
            curr_back: Some(range.end - 1),
            range,
        }
    }

    /// derives the nth child of the parent-key.
    ///
    /// This is different from Iterator::nth() because:
    /// 1. that method is relative to the current iter position rather
    ///    than the parent-key.
    /// 2. that method iterates over all elems up to n, whereas this method
    ///    only derives the requested key, so is much faster when index > 0.
    pub fn derive_child(&self, index: DerivationIndex) -> SpendingKey {
        self.parent_key.derive_child(index)
    }
}
impl Iterator for SpendingKeyIter {
    type Item = SpendingKey;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.curr, self.curr_back) {
            (Some(c), Some(cb)) => {
                let key = self.parent_key.derive_child(c);
                self.curr = if c >= cb { None } else { Some(c + 1) };
                Some(key)
            }
            _ => None,
        }
    }

    // returns a tuple where the first element is the lower bound, and the
    // second element is the upper bound
    //
    // note: the cast to usize should always succeed because we already
    // assert that range len is <= usize::MAX when iterator is created.
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.range.end - self.range.start) as usize;
        (len, Some(len))
    }
}

// rayon needs DoubleEndedIterator, bleah.
// see: https://github.com/rayon-rs/rayon/issues/1053
impl DoubleEndedIterator for SpendingKeyIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        match (self.curr, self.curr_back) {
            (Some(c), Some(cb)) => {
                let key = self.parent_key.derive_child(cb);
                self.curr_back = if cb <= c { None } else { Some(cb - 1) };
                Some(key)
            }
            _ => None,
        }
    }
}

// note: Iterator::size_hint() must return exact size
impl ExactSizeIterator for SpendingKeyIter {}

pub mod par_iter {
    use rayon::iter::plumbing::bridge;
    use rayon::iter::plumbing::Consumer;
    use rayon::iter::plumbing::Producer;
    use rayon::iter::plumbing::ProducerCallback;
    use rayon::iter::plumbing::UnindexedConsumer;
    use rayon::prelude::IndexedParallelIterator;
    use rayon::prelude::ParallelIterator;

    use super::*;

    pub struct SpendingKeyParallelIter(SpendingKeyIter);

    impl From<SpendingKeyIter> for SpendingKeyParallelIter {
        fn from(iter: SpendingKeyIter) -> Self {
            Self(iter)
        }
    }

    impl ParallelIterator for SpendingKeyParallelIter {
        type Item = SpendingKey;

        fn drive_unindexed<C>(self, consumer: C) -> C::Result
        where
            C: UnindexedConsumer<Self::Item>,
        {
            bridge(self, consumer)
        }

        fn opt_len(&self) -> Option<usize> {
            Some(ExactSizeIterator::len(&self.0))
        }
    }

    impl IndexedParallelIterator for SpendingKeyParallelIter {
        fn with_producer<CB: ProducerCallback<Self::Item>>(self, callback: CB) -> CB::Output {
            callback.callback(SpendingKeyRangeProducer::from(self))
        }

        fn drive<C: Consumer<Self::Item>>(self, consumer: C) -> C::Result {
            bridge(self, consumer)
        }

        fn len(&self) -> usize {
            ExactSizeIterator::len(&self.0)
        }
    }

    struct SpendingKeyRangeProducer(SpendingKeyParallelIter);

    impl Producer for SpendingKeyRangeProducer {
        type Item = SpendingKey;
        type IntoIter = SpendingKeyIter;

        fn into_iter(self) -> Self::IntoIter {
            self.0 .0
        }

        fn split_at(self, index: usize) -> (Self, Self) {
            let range_iter = self.0 .0;

            let mid = range_iter.range.start + index as DerivationIndex;

            let left = SpendingKeyIter::new_range(
                range_iter.parent_key,
                range_iter.range.start..(mid - 1) as DerivationIndex,
            );
            let right =
                SpendingKeyIter::new_range(range_iter.parent_key, mid..range_iter.range.end);
            (
                Self(SpendingKeyParallelIter(left)),
                Self(SpendingKeyParallelIter(right)),
            )
        }
    }

    impl From<SpendingKeyParallelIter> for SpendingKeyRangeProducer {
        fn from(range_iter: SpendingKeyParallelIter) -> Self {
            Self(range_iter)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::state::wallet::symmetric_key::SymmetricKey;

    mod iter_tests {
        use super::*;

        // tests that ::derive_nth() matches ::next()
        #[test]
        pub fn derive_nth_matches_iter() {
            worker::derive_nth_matches_iter();
        }

        // tests basic iteration, comparing with SpendingKey::derive_child()
        #[test]
        pub fn iterator() {
            let parent_key = helper::make_parent_key();
            worker::iterator(parent_key, parent_key.into_iter());
        }

        // tests basic iteration over a range, comparing with SpendingKey::derive_child()
        #[test]
        pub fn range_iterator() {
            let parent_key = helper::make_parent_key();
            worker::iterator(parent_key, parent_key.into_range_iter(0..50));
        }

        // tests Iterator::nth() method, comparing with SpendingKey::derive_child()
        #[test]
        pub fn iterator_nth() {
            let parent_key = helper::make_parent_key();
            worker::iterator_nth(parent_key, parent_key.into_iter());
        }

        // tests Iterator::nth() method for a range, comparing with SpendingKey::derive_child()
        #[test]
        pub fn range_iterator_nth() {
            let parent_key = helper::make_parent_key();
            worker::iterator_nth(parent_key, parent_key.into_range_iter(0..50));
        }

        // tests that a range iterator reaches last elem and after returns None
        #[test]
        pub fn range_iterator_to_last_elem() {
            let parent_key = helper::make_parent_key();

            let first = 0;
            let len = 50;
            worker::iterator_to_last_elem(
                parent_key,
                parent_key.into_range_iter(first..first + len),
                first,
                len,
            );
        }

        // tests that iterator can reach DerivationIndex::MAX and after returns None
        #[test]
        pub fn iterator_to_max_elem() {
            let parent_key = helper::make_parent_key();

            let last = DerivationIndex::MAX;
            let len = 10;
            let first = last - len;
            worker::iterator_to_last_elem(
                parent_key,
                parent_key.into_range_iter(first..last),
                first,
                len,
            );
        }

        // tests that iterator operates in reverse
        #[test]
        pub fn double_ended_iterator() {
            let parent_key = helper::make_parent_key();
            worker::double_ended_iterator(
                parent_key,
                parent_key.into_iter(),
                usize::MAX as DerivationIndex,
            );
        }

        // tests that range iterator operates in reverse
        #[test]
        pub fn double_ended_range_iterator() {
            let parent_key = helper::make_parent_key();
            let len = 50;
            worker::double_ended_iterator(parent_key, parent_key.into_range_iter(0..len), len);
        }

        // tests that forward and reverse iteration meets in the middle and do
        // not pass eachother.
        #[test]
        pub fn double_ended_iterator_meet_middle() {
            let parent_key = helper::make_parent_key();

            let len = 50;
            worker::double_ended_iterator_meet_middle(
                parent_key,
                parent_key.into_range_iter(0..len),
                len,
            );
        }

        /// tests that reverse iteration does not go past first elem in range
        #[test]
        pub fn double_ended_iterator_to_first_elem() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            let first = 10;
            let len = 20;
            worker::double_ended_iterator_to_first_elem(
                parent_key,
                parent_key.into_range_iter(first..first + len),
                first,
                len,
            );
        }

        // tests that reverse iteration can reach 0 elem, and stops after
        #[test]
        pub fn double_ended_iterator_to_zero_elem() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            let first = 0;
            let len = 20;
            worker::double_ended_iterator_to_first_elem(
                parent_key,
                parent_key.into_range_iter(first..len),
                first,
                len,
            );
        }

        mod worker {
            use super::*;

            // derives 5 keys and verifies that:
            //  1. derive_nth(n) matches Iterator::next() for each
            //  2. derive_nth() works (from parent_key) even after iteration has begun.
            pub fn derive_nth_matches_iter() {
                let mut iter = helper::make_iter();

                for n in 0..5 {
                    assert_eq!(Some(iter.derive_child(n)), iter.next());
                }
            }

            // derives 5 keys and verifies that iteration results match SpendingKey::derive_child() results
            pub fn iterator(parent_key: SpendingKey, mut iter: impl Iterator<Item = SpendingKey>) {
                for n in 0..5 {
                    assert_eq!(Some(parent_key.derive_child(n)), iter.next());
                }
            }

            // verifies that Iterator::nth() works and does not rewind the iterator
            pub fn iterator_nth(
                parent_key: SpendingKey,
                mut iter: impl Iterator<Item = SpendingKey>,
            ) {
                assert_eq!(Some(parent_key.derive_child(5)), iter.nth(5));

                // verify that nth() does not rewind iterator.
                assert_eq!(Some(parent_key.derive_child(6)), iter.nth(0));
            }

            // tests that iteration reaches last elem in range and then stops.
            pub fn iterator_to_last_elem(
                parent_key: SpendingKey,
                mut iter: impl Iterator<Item = SpendingKey>,
                start: DerivationIndex,
                len: DerivationIndex,
            ) {
                assert_eq!(
                    Some(parent_key.derive_child(start + len - 1)),
                    iter.nth((len - 1) as usize)
                );

                assert_eq!(None, iter.next());
            }

            // tests that backwards iteration works
            pub fn double_ended_iterator(
                parent_key: SpendingKey,
                mut iter: impl DoubleEndedIterator<Item = SpendingKey>,
                len: DerivationIndex,
            ) {
                for n in 0..5 {
                    assert_eq!(Some(parent_key.derive_child(n)), iter.next());
                }
                for n in (len - 5..len).rev() {
                    assert_eq!(Some(parent_key.derive_child(n)), iter.next_back());
                }
            }

            // tests that forward and backwards iteration meets in the middle
            // and do not pass eachother
            pub fn double_ended_iterator_meet_middle(
                parent_key: SpendingKey,
                mut iter: impl DoubleEndedIterator<Item = SpendingKey>,
                len: DerivationIndex,
            ) {
                for n in 0..5 {
                    assert_eq!(Some(parent_key.derive_child(n)), iter.next());
                }
                assert_eq!(
                    Some(parent_key.derive_child(10)),
                    iter.nth_back((len - 1 - 10) as usize)
                );

                for n in (5..10).rev() {
                    assert_eq!(Some(parent_key.derive_child(n)), iter.next_back());
                }

                assert_eq!(None, iter.next_back());
                assert_eq!(None, iter.next());
            }

            // tests that backwards iteration can reach the first element in a range.
            pub fn double_ended_iterator_to_first_elem(
                parent_key: SpendingKey,
                mut iter: impl DoubleEndedIterator<Item = SpendingKey>,
                first: DerivationIndex,
                len: DerivationIndex,
            ) {
                assert_eq!(
                    Some(parent_key.derive_child(first + 1)),
                    iter.nth_back((len - 2) as usize)
                );

                assert_eq!(Some(parent_key.derive_child(first)), iter.next_back());
                assert_eq!(None, iter.next_back());
            }
        }
    }

    mod par_iter_tests {
        use super::*;

        // tests iteration over entire range, comparing with SpendingKey::derive_child()
        #[test]
        pub fn range_iterator_entire_range() {
            worker::iterator_all_in_range(10, 500);
        }

        mod worker {
            use std::collections::HashSet;

            use rayon::iter::IndexedParallelIterator;
            use rayon::iter::ParallelIterator;

            use super::*;

            // compares parallel range iter results to non-parallel range iter results.
            pub fn iterator_all_in_range(start: DerivationIndex, end: DerivationIndex) {
                let parent_key = helper::make_parent_key();
                let set1: HashSet<SpendingKey> = parent_key.into_range_iter(start..end).collect();

                // test by collect() and set equality.
                let set2: HashSet<SpendingKey> =
                    parent_key.into_par_range_iter(start..end).collect();
                assert_eq!(set1, set2);

                // test without collect(), by comparing len, and ensuring all elems are in the set.
                let par_iter = parent_key.into_par_range_iter(start..end);
                assert!(par_iter.len() == set1.len());
                assert!(par_iter.all(|k| set1.contains(&k)));
            }
        }
    }

    mod helper {
        use super::*;

        // generates a random SpendingKey.
        //
        // note: we use a SymmetricKey because it is faster to derive and uses
        // less memory than GenerationSpendingKey.  For purposes of testing iterator
        // logic, the key-type does not matter.
        pub fn make_parent_key() -> SpendingKey {
            SymmetricKey::from_seed(rand::random()).into()
        }

        // generates an iterator for a random SpendingKey::SymmetricKey
        pub fn make_iter() -> SpendingKeyIter {
            make_parent_key().into_iter()
        }
    }
}
