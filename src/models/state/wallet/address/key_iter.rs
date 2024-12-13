use serde::Deserialize;
use serde::Serialize;

use super::DerivationIndex;
use super::SpendingKey;

// an iterator over a range of spending keys
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingKeyIter {
    parent_key: SpendingKey,
    first: DerivationIndex,
    last: DerivationIndex,
    curr: Option<DerivationIndex>,
    curr_back: Option<DerivationIndex>,
}

impl SpendingKeyIter {
    pub fn new(parent_key: SpendingKey) -> Self {
        Self::new_range(parent_key, 0, DerivationIndex::MAX)
    }

    pub fn new_range(
        parent_key: SpendingKey,
        first: DerivationIndex,
        last: DerivationIndex,
    ) -> Self {
        Self {
            parent_key,
            first,
            last,
            curr: Some(first),
            curr_back: Some(last),
        }
    }

    pub fn derive_nth(&self, index: DerivationIndex) -> SpendingKey {
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
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len: usize = (self.last - self.first) as usize;
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

mod par_iter {
    use rayon::iter::plumbing::bridge;
    use rayon::iter::plumbing::Consumer;
    use rayon::iter::plumbing::Producer;
    use rayon::iter::plumbing::ProducerCallback;
    use rayon::iter::plumbing::UnindexedConsumer;
    use rayon::prelude::IndexedParallelIterator;
    use rayon::prelude::ParallelIterator;

    use super::*;

    impl ParallelIterator for SpendingKeyIter {
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

    impl IndexedParallelIterator for SpendingKeyIter {
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

    struct SpendingKeyRangeProducer(SpendingKeyIter);

    impl Producer for SpendingKeyRangeProducer {
        type Item = SpendingKey;
        type IntoIter = SpendingKeyIter;

        fn into_iter(self) -> Self::IntoIter {
            self.0
        }

        fn split_at(self, index: usize) -> (Self, Self) {
            let range_iter = self.0;

            let left = SpendingKeyIter::new_range(
                range_iter.parent_key,
                range_iter.first,
                (index - 1) as DerivationIndex,
            );
            let right = SpendingKeyIter::new_range(
                range_iter.parent_key,
                index as DerivationIndex,
                range_iter.last,
            );
            (Self(left), Self(right))
        }
    }

    impl From<SpendingKeyIter> for SpendingKeyRangeProducer {
        fn from(range_iter: SpendingKeyIter) -> Self {
            Self(range_iter)
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
        pub fn iterator_nth() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            worker::iterator_nth(parent_key, parent_key.into_iter());
        }

        #[test]
        pub fn range_iterator_nth() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            worker::iterator_nth(parent_key, parent_key.into_range_iter(0, 50));
        }

        #[test]
        pub fn range_iterator_to_last_elem() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            let len = 50;
            worker::iterator_to_last_elem(parent_key, parent_key.into_range_iter(0, len), len);
        }

        #[test]
        pub fn double_ended_iterator() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            worker::double_ended_iterator(parent_key, parent_key.into_iter(), DerivationIndex::MAX);
        }

        #[test]
        pub fn double_ended_range_iterator() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            let len = 50;
            worker::double_ended_iterator(parent_key, parent_key.into_range_iter(0, len), len);
        }

        // #[test]
        // pub fn double_ended_iterator_meet_middle() {
        //     let parent_key = SymmetricKey::from_seed(rand::random()).into();

        //     worker::double_ended_iterator_meet_middle(parent_key, parent_key.into_iter(), DerivationIndex::MAX);
        // }

        #[test]
        pub fn double_ended_range_iterator_meet_middle() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            let len = 50;
            worker::double_ended_iterator_meet_middle(
                parent_key,
                parent_key.into_range_iter(0, len),
                len,
            );
        }

        #[test]
        pub fn double_ended_range_iterator_to_first_elem() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            let len = 50;
            worker::double_ended_iterator_to_first_elem(
                parent_key,
                parent_key.into_range_iter(0, len),
                len,
            );
        }

        mod worker {
            use super::*;

            pub fn iterator(parent_key: SpendingKey, mut iter: impl Iterator<Item = SpendingKey>) {
                for n in 0..5 {
                    assert_eq!(Some(parent_key.derive_child(n)), iter.next());
                }
            }

            pub fn iterator_nth(
                parent_key: SpendingKey,
                mut iter: impl Iterator<Item = SpendingKey>,
            ) {
                assert_eq!(Some(parent_key.derive_child(5)), iter.nth(5));

                // verify that nth() does not rewind iterator.
                assert_eq!(Some(parent_key.derive_child(6)), iter.nth(0));
            }

            pub fn iterator_to_last_elem(
                parent_key: SpendingKey,
                mut iter: impl Iterator<Item = SpendingKey>,
                len: DerivationIndex,
            ) {
                assert_eq!(
                    Some(parent_key.derive_child(len - 1)),
                    iter.nth((len - 1) as usize)
                );

                assert_eq!(Some(parent_key.derive_child(len)), iter.next());
                assert_eq!(None, iter.next());
            }

            pub fn double_ended_iterator(
                parent_key: SpendingKey,
                mut iter: impl DoubleEndedIterator<Item = SpendingKey>,
                len: DerivationIndex,
            ) {
                for n in 0..5 {
                    assert_eq!(Some(parent_key.derive_child(n)), iter.next());
                }
                for n in (len - 5..=len).rev() {
                    assert_eq!(Some(parent_key.derive_child(n)), iter.next_back());
                }
            }

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
                    iter.nth_back((len - 10) as usize)
                );

                for n in (5..10).rev() {
                    assert_eq!(Some(parent_key.derive_child(n)), iter.next_back());
                }

                assert_eq!(None, iter.next_back());
                assert_eq!(None, iter.next());
            }

            pub fn double_ended_iterator_to_first_elem(
                parent_key: SpendingKey,
                mut iter: impl DoubleEndedIterator<Item = SpendingKey>,
                len: DerivationIndex,
            ) {
                assert_eq!(
                    Some(parent_key.derive_child(1)),
                    iter.nth_back((len - 1) as usize)
                );

                assert_eq!(Some(parent_key.derive_child(0)), iter.next_back());

                assert_eq!(None, iter.next_back());
            }
        }
    }

    mod par_iter {}
}
