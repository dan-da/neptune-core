use db_key::Key;
use serde::Serialize;
use twenty_first::shared_math::{b_field_element::BFieldElement, traits::FromVecu8};

pub const BYTES_PER_BFE: usize = 8;
pub const RESCUE_PRIME_OUTPUT_SIZE_IN_BFES: usize = 6;
pub const RESCUE_PRIME_DIGEST_SIZE_IN_BYTES: usize =
    RESCUE_PRIME_OUTPUT_SIZE_IN_BFES * BYTES_PER_BFE;

// The data structure `RescuePrimeDigest` is primarily needed, so we can make
// database keys out of rescue prime digests.
#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize, PartialEq)]
pub struct RescuePrimeDigest([BFieldElement; RESCUE_PRIME_OUTPUT_SIZE_IN_BFES]);

// Digest needs a partial ordering for the mining/PoW process, to check if
// a digest is below the difficulty threshold.
impl PartialOrd for RescuePrimeDigest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        for i in 0..RESCUE_PRIME_OUTPUT_SIZE_IN_BFES {
            if self.0[i].value() != other.0[i].value() {
                return self.0[i].value().partial_cmp(&other.0[i].value());
            }
        }

        return None;
    }
}

impl RescuePrimeDigest {
    pub fn values(&self) -> [BFieldElement; RESCUE_PRIME_OUTPUT_SIZE_IN_BFES] {
        self.0
    }

    pub fn new(digest: [BFieldElement; RESCUE_PRIME_OUTPUT_SIZE_IN_BFES]) -> Self {
        Self(digest)
    }

    pub fn default() -> Self {
        Self([BFieldElement::ring_zero(); RESCUE_PRIME_OUTPUT_SIZE_IN_BFES])
    }
}

impl From<[u8; RESCUE_PRIME_DIGEST_SIZE_IN_BYTES]> for RescuePrimeDigest {
    fn from(item: [u8; RESCUE_PRIME_DIGEST_SIZE_IN_BYTES]) -> Self {
        let mut bfes: [BFieldElement; RESCUE_PRIME_OUTPUT_SIZE_IN_BFES] =
            [BFieldElement::ring_zero(); RESCUE_PRIME_OUTPUT_SIZE_IN_BFES];
        for i in 0..RESCUE_PRIME_OUTPUT_SIZE_IN_BFES {
            let start_index = i * RESCUE_PRIME_DIGEST_SIZE_IN_BYTES;
            let end_index = (i + 1) * RESCUE_PRIME_DIGEST_SIZE_IN_BYTES;
            bfes[i] = BFieldElement::ring_zero().from_vecu8(item[start_index..end_index].to_vec())
        }

        Self(bfes)
    }
}

impl From<[BFieldElement; RESCUE_PRIME_OUTPUT_SIZE_IN_BFES]> for RescuePrimeDigest {
    fn from(array: [BFieldElement; RESCUE_PRIME_OUTPUT_SIZE_IN_BFES]) -> Self {
        RescuePrimeDigest(array)
    }
}

impl From<RescuePrimeDigest> for [BFieldElement; RESCUE_PRIME_OUTPUT_SIZE_IN_BFES] {
    fn from(val: RescuePrimeDigest) -> Self {
        val.0
    }
}

impl From<Vec<BFieldElement>> for RescuePrimeDigest {
    fn from(elems: Vec<BFieldElement>) -> Self {
        let argument_length = elems.len();
        let array: [BFieldElement; RESCUE_PRIME_OUTPUT_SIZE_IN_BFES] =
            elems.try_into().expect(&format!(
                "Digest must have length {}. Got: {}",
                RESCUE_PRIME_OUTPUT_SIZE_IN_BFES, argument_length,
            ));

        array.into()
    }
}

impl From<RescuePrimeDigest> for Vec<BFieldElement> {
    fn from(val: RescuePrimeDigest) -> Self {
        val.0.to_vec()
    }
}

impl Key for RescuePrimeDigest {
    fn from_u8(key: &[u8]) -> Self {
        let converted_key: [u8; RESCUE_PRIME_DIGEST_SIZE_IN_BYTES] = key
            .to_owned()
            .try_into()
            .expect("slice with incorrect length used as block hash");
        converted_key.into()
    }

    fn as_slice<T, F: Fn(&[u8]) -> T>(&self, f: F) -> T {
        let u8s: [u8; RESCUE_PRIME_DIGEST_SIZE_IN_BYTES] = self.to_owned().into();
        f(&u8s)
    }
}

impl From<RescuePrimeDigest> for [u8; RESCUE_PRIME_DIGEST_SIZE_IN_BYTES] {
    fn from(item: RescuePrimeDigest) -> Self {
        let u64s = item.0.iter().map(|x| x.value());
        u64s.map(|x| x.to_ne_bytes())
            .collect::<Vec<_>>()
            .concat()
            .try_into()
            .unwrap()
    }
}

#[cfg(test)]
mod digest_tests {
    use super::*;

    #[test]
    fn digest_ordering() {
        let val0 =
            RescuePrimeDigest::new([BFieldElement::new(0); RESCUE_PRIME_OUTPUT_SIZE_IN_BFES]);
        let val1 = RescuePrimeDigest::new([
            BFieldElement::new(14),
            BFieldElement::new(0),
            BFieldElement::new(0),
            BFieldElement::new(0),
            BFieldElement::new(0),
            BFieldElement::new(0),
        ]);
        assert!(val0 < val1);

        let val2 =
            RescuePrimeDigest::new([BFieldElement::new(14); RESCUE_PRIME_OUTPUT_SIZE_IN_BFES]);
        assert!(val2 > val1);
        assert!(val2 > val0);

        let val3 = RescuePrimeDigest::new([
            BFieldElement::new(14),
            BFieldElement::new(14),
            BFieldElement::new(14),
            BFieldElement::new(14),
            BFieldElement::new(14),
            BFieldElement::new(15),
        ]);
        assert!(val3 > val2);
        assert!(val3 > val1);
        assert!(val3 > val0);

        let val4 = RescuePrimeDigest::new([
            BFieldElement::new(14),
            BFieldElement::new(14),
            BFieldElement::new(14),
            BFieldElement::new(14),
            BFieldElement::new(15),
            BFieldElement::new(14),
        ]);
        assert!(val4 > val3);
        assert!(val4 > val2);
        assert!(val4 > val1);
        assert!(val4 > val0);
    }
}
