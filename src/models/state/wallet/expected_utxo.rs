use crate::config_models::network::Network;
use crate::models::blockchain::{shared::Hash, transaction::utxo::Utxo};
use crate::{
    models::consensus::timestamp::Timestamp,
    prelude::twenty_first,
    util_types::mutator_set::{addition_record::AdditionRecord, commit},
};
use anyhow::bail;
use anyhow::Result;
use bech32::FromBase32;
use bech32::ToBase32;
use get_size::GetSize;
use serde::{Deserialize, Serialize};
use tasm_lib::triton_vm::prelude::BFieldElement;
use twenty_first::{math::tip5::Digest, util_types::algebraic_hasher::AlgebraicHasher};

use super::address::{ReceivingAddress, SpendingKey};

/// represents utxo and secrets necessary for recipient to claim it.
///
/// [ExpectedUtxo] is intended for offchain temporary storage of utxos that a
/// wallet sends to itself, eg change outputs.
///
/// The `ExpectedUtxo` will exist in the local
/// [RustyWalletDatabase](super::rusty_wallet_database::RustyWalletDatabase)
/// from the time the transaction is sent until it is mined in a block and
/// claimed by the wallet.
///
/// note that when using `ExpectedUtxo` there is a risk of losing funds because
/// the wallet stores this state on disk and if the associated file(s) are lost
/// then the funds cannot be claimed.
///
/// an alternative is to use onchain symmetric keys instead, which uses some
/// blockchain space and may leak some privacy if a key is ever used more than
/// once.
///
/// ### about `receiver_preimage`
///
/// See issue #176.
/// <https://github.com/Neptune-Crypto/neptune-core/issues/176>
///
/// see [AnnouncedUtxo](crate::models::blockchain::transaction::AnnouncedUtxo), [UtxoNotification](crate::models::blockchain::transaction::UtxoNotification)
#[derive(Clone, Debug, PartialEq, Eq, Hash, GetSize, Serialize, Deserialize)]
pub struct ExpectedUtxo {
    pub utxo: Utxo,
    pub addition_record: AdditionRecord,
    pub sender_randomness: Digest,
    pub receiver_preimage: Digest,
    pub received_from: UtxoNotifier,
    pub notification_received: Timestamp,
    pub mined_in_block: Option<(Digest, Timestamp)>,
}

impl ExpectedUtxo {
    pub fn new(
        utxo: Utxo,
        sender_randomness: Digest,
        receiver_preimage: Digest,
        received_from: UtxoNotifier,
    ) -> Self {
        Self {
            addition_record: commit(
                Hash::hash(&utxo),
                sender_randomness,
                receiver_preimage.hash::<Hash>(),
            ),
            utxo,
            sender_randomness,
            receiver_preimage,
            received_from,
            notification_received: Timestamp::now(),
            mined_in_block: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, GetSize, Serialize, Deserialize)]
pub enum UtxoNotifier {
    OwnMiner,
    Cli,
    Myself,
    Premine,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, GetSize, Serialize, Deserialize)]
pub struct UtxoTransfer {
    pub utxo: Utxo,
    pub sender_randomness: Digest,
}

impl UtxoTransfer {
    pub fn new(utxo: Utxo, sender_randomness: Digest) -> Self {
        Self {
            utxo,
            sender_randomness,
        }
    }

    pub fn encrypt_to_address(
        &self,
        address: &ReceivingAddress,
    ) -> anyhow::Result<UtxoTransferEncrypted> {
        Ok(UtxoTransferEncrypted {
            ciphertext: address.encrypt(&self.utxo, self.sender_randomness)?,
            receiver_identifier: address.receiver_identifier(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, GetSize, Serialize, Deserialize)]
pub struct UtxoTransferEncrypted {
    pub ciphertext: Vec<BFieldElement>,
    pub receiver_identifier: BFieldElement,
}

impl UtxoTransferEncrypted {
    pub fn decrypt_with_spending_key(
        &self,
        spending_key: &SpendingKey,
    ) -> anyhow::Result<UtxoTransfer> {
        let (utxo, sender_randomness) = spending_key.decrypt(&self.ciphertext)?;

        Ok(UtxoTransfer {
            utxo,
            sender_randomness,
        })
    }

    pub fn to_bech32m(&self, network: Network) -> Result<String> {
        let hrp = Self::get_hrp(network);
        let payload = bincode::serialize(self)?;
        let variant = bech32::Variant::Bech32m;
        match bech32::encode(hrp, payload.to_base32(), variant) {
            Ok(enc) => Ok(enc),
            Err(e) => bail!("Could not encode UtxoTransferEncrypted as bech32m because error: {e}"),
        }
    }

    pub fn from_bech32m(encoded: &str, network: Network) -> Result<Self> {
        let (hrp, data, variant) = bech32::decode(encoded)?;

        if variant != bech32::Variant::Bech32m {
            bail!("Can only decode bech32m addresses.");
        }

        if hrp[0..=4] != *Self::get_hrp(network) {
            bail!("Could not decode bech32m address because of invalid prefix");
        }

        let payload = Vec::<u8>::from_base32(&data)?;

        match bincode::deserialize(&payload) {
            Ok(ra) => Ok(ra),
            Err(e) => bail!("Could not decode bech32m because of error: {e}"),
        }
    }

    pub fn get_hrp(_network: Network) -> &'static str {
        "utxo"
    }
}
