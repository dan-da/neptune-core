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
        println!("curr: {:?}, curr_back: {:?}", self.curr, self.curr_back);
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
            self.0.0
        }

        fn split_at(self, index: usize) -> (Self, Self) {
            let range_iter = self.0.0;

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

    mod iter {
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
            worker::iterator(parent_key, parent_key.into_range_iter(0, 50));
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
            worker::iterator_nth(parent_key, parent_key.into_range_iter(0, 50));
        }

        // tests that a range iterator reaches last elem and after returns None
        #[test]
        pub fn range_iterator_to_last_elem() {
            let parent_key = helper::make_parent_key();

            let first = 0;
            let len = 50;
            worker::iterator_to_last_elem(
                parent_key,
                parent_key.into_range_iter(first, first + len),
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
                parent_key.into_range_iter(first, last),
                first,
                len,
            );
        }

        // tests that iterator operates in reverse
        #[test]
        pub fn double_ended_iterator() {
            let parent_key = helper::make_parent_key();
            worker::double_ended_iterator(parent_key, parent_key.into_iter(), DerivationIndex::MAX);
        }

        // tests that range iterator operates in reverse
        #[test]
        pub fn double_ended_range_iterator() {
            let parent_key = helper::make_parent_key();
            let len = 50;
            worker::double_ended_iterator(parent_key, parent_key.into_range_iter(0, len), len);
        }

        // tests that forward and reverse iteration meets in the middle and do
        // not pass eachother.
        #[test]
        pub fn double_ended_iterator_meet_middle() {
            let parent_key = helper::make_parent_key();

            let len = 50;
            worker::double_ended_iterator_meet_middle(
                parent_key,
                parent_key.into_range_iter(0, len),
                len,
            );
        }

        // tests that reverse iteration does not go past first elem in range
        #[test]
        pub fn double_ended_iterator_to_first_elem() {
            let parent_key = SymmetricKey::from_seed(rand::random()).into();

            let first = 10;
            let len = 20;
            worker::double_ended_iterator_to_first_elem(
                parent_key,
                parent_key.into_range_iter(first, first + len),
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
                parent_key.into_range_iter(first, len),
                first,
                len,
            );
        }
    }

    mod par_iter {
        use super::*;
        use rayon::iter::IntoParallelIterator;
/*
        // tests basic iteration, comparing with SpendingKey::derive_child()
        #[test]
        pub fn iterator() {
            let parent_key = helper::make_parent_key();
            worker::iterator(parent_key, parent_key.into_par_iter());
        }

        // tests basic iteration over a range, comparing with SpendingKey::derive_child()
        #[test]
        pub fn range_iterator() {
            let parent_key = helper::make_parent_key();
            worker::iterator(parent_key, parent_key.into_par_range_iter(0, 50));
        }
*/
        // tests iteration over entire range, comparing with SpendingKey::derive_child()
        #[test]
        pub fn range_iterator_entire_range() {
            use rayon::iter::ParallelBridge;
            let parent_key = helper::make_parent_key();
            worker::iterator_all(parent_key, parent_key.into_par_range_iter(10, 50000000), 10);
            //worker::iterator_all(parent_key, parent_key.into_range_iter(10, 50000000).par_bridge(), 10);
        }
/*
        // tests Iterator::nth() method, comparing with SpendingKey::derive_child()
        #[test]
        pub fn iterator_nth() {
            let parent_key = helper::make_parent_key();
            worker::iterator_nth(parent_key, parent_key.into_par_iter());
        }

        // tests Iterator::nth() method for a range, comparing with SpendingKey::derive_child()
        #[test]
        pub fn range_iterator_nth() {
            let parent_key = helper::make_parent_key();
            worker::iterator_nth(parent_key, parent_key.into_par_range_iter(0, 50));
        }
*/
    }

    mod helper {
        use super::*;

        pub fn make_parent_key() -> SpendingKey {
            SymmetricKey::from_seed(rand::random()).into()
        }

        pub fn make_iter() -> SpendingKeyIter {
            make_parent_key().into_iter()
        }

        // pub fn make_range_iter(start: DerivationIndex, end: DerivationIndex) -> SpendingKeyIter {
        //     SpendingKeyIter::new_range(make_parent_key(), start, end)
        // }
    }

    mod worker {
        use super::*;
        use rayon::iter::IterBridge;
        use rayon::iter::ParallelIterator;

        pub fn derive_nth_matches_iter() {
            let mut iter = helper::make_iter();

            for n in 0..5 {
                assert_eq!(Some(iter.derive_nth(n)), iter.next());
            }
        }

        pub fn iterator(parent_key: SpendingKey, mut iter: impl Iterator<Item = SpendingKey>) {
            for n in 0..5 {
                assert_eq!(Some(parent_key.derive_child(n)), iter.next());
            }
        }

        pub fn iterator_all(parent_key: SpendingKey, iter: super::super::super::par_iter::SpendingKeyParallelIter, start: DerivationIndex) {
            iter.for_each(|key| println!("another"));
        }


        pub fn iterator_nth(parent_key: SpendingKey, mut iter: impl Iterator<Item = SpendingKey>) {
            assert_eq!(Some(parent_key.derive_child(5)), iter.nth(5));

            // verify that nth() does not rewind iterator.
            assert_eq!(Some(parent_key.derive_child(6)), iter.nth(0));
        }

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

            assert_eq!(Some(parent_key.derive_child(start + len)), iter.next());
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
            first: DerivationIndex,
            len: DerivationIndex,
        ) {
            assert_eq!(
                Some(parent_key.derive_child(first + 1)),
                iter.nth_back((len - 1) as usize)
            );

            assert_eq!(Some(parent_key.derive_child(first)), iter.next_back());
            assert_eq!(None, iter.next_back());
        }
    }
}
