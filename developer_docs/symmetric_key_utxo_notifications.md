# Symmetric Key Utxo Notifications:  Planning

## Background

When a UTXO is created by a sender it is necessary to somehow transmit certain
secrets to the recipient, so that the recipient can claim the UTXO.

There are primarily two cases:
 1. sender and recipient are different wallets
 2. sender and recipient are the same wallet

For case 1 we have public-key `GenerationAddress` that generate
`PublicAnnouncements` by encrypting the secrets to the recipient's known
public-key. These `PublicAnnouncements` are stored on the blockchain for the
recipient.  This works well but the encrypted data is somewhat large, taking up
valuable blockchain space.

For case 2, we presently have `ExpectedUtxos` which stores the secrets in the
local wallet after a transaction is sent so that the wallet can claim the Utxo
once the transaction has been confirmed in a block.  This has the advantage that
no blockchain space is used and the secrets are never put anywhere public even
in encrypted form.  However it means that:

* A wallet cannot be reconstructed from seed alone.

* If wallet is lost and no backup exists, funds will probably be lost,
especially from change addresses.  Even though user still has the
seed/mnemonic.

* Device independence is lost/diminished.

Given the limitations of (2), it is desirable to implement a mechanism that
places the encrypted secrets on-chain but does not use as much space as the
public-key encrypted ciphertext.  It is proposed that we use symmetric key
cryptography for this.   The rest of this document describes how this can be
achieved.

## Ciphertext length: Symmetric vs Asymmetric Keys

Given that saving blockchain space is a primary motivator, it would be helpful
to quantitatively understand the difference in ciphertext length for the same
data encrypted with our existing Asymmetric algorithm vs any proposed Symmetric
Key algorithm.

A quick internet search did not turn up meaningful results.  I did find a
statement indicating that:

* Symmetric-key ciphertext length is the same or smaller than the original plaintext.

* Asymmetric-key ciphertext length is the same or larger than the original plaintext.

Other sources indicate that symmetric key ciphertext length is normally the same
or a tiny bit larger than the plaintext, due to padding.

So then, if our asymmetric ciphertext is also near the same size as the plaintext,
we may not be saving any/much blockchain space by using symmetric keys.

Let's perform a test:

TODO: test results.

If we cannot obtain significant improvements in ciphertext length, then it is
unclear if there is sufficient value in implementing symmetric keys rather than
simply using the existing asymmetric keys for this purpose.  Symmetric keys are
smaller and offer faster encryption and decryption, however those advantages do
not help save any blockchain space.  Also, we would need to implement and
maintain two keys systems instead of just one.

Alan: perhaps you can provide some perspective here?


## UtxoNotifyMethod

We already have `PublicAnnouncements` and `ExpectedUtxos`.  It seems clearer to
call these `OnChain` and `OffChain` notifications, respectively.

A recent PR has introduced the following enum:

```
pub enum UtxoNotifyMethod {
    OnChainPubKey(PublicAnnouncement),
    OnChainSymmetricKey,             // --> PublicAnnouncement
    OffChain,                        // --> ExpectedUtxo
}
```

There are two OnChain mechanisms, and one OffChain mechanism.

### Example

When Alice sends to her own wallet, including receiving change, she can choose
between `OnChainSymmetricKey` or `OffChain`.

When Alice sends to Bob's wallet, she must use `OnChainPubKey`.

### Alternate Construction

It might be semantically clearer to do something like this:

```
pub enum UtxoOnChainNotifyMethod {
    PubKey(PublicAnnouncement),
    SymmetricKey(PublicAnnouncement),
}

pub enum UtxoNotifyMethod {
    OnChain(UtxoOnChainNotifyMethod),
    OffChain(ExpectedUtxo),
}
```

### Naming consistency

It might make sense to rename `PublicAnnouncement` and `ExpectedUtxo` to match the
above enums.  So it could look like:

```
// PublicAnnouncment --> UtxoOnChainNotification
// ExpectedUtxo      --> UtxoOffChainNotification

pub enum UtxoOnChainNotifyMethod {
    PubKey(UtxoOnChainNotification)
    SymmetricKey(UtxoOnChainNotification)
}

pub enum UtxoNotifyMethod {
    OnChain(UtxoOnChainNotifyMethod),
    OffChain(UtxoOffChainNotification),
}
```

## Symmetric Key Cipher Selection

TODO: More work/research needed here.

Some possible candidates:  AES, RC6, Twofish, SPECK128, LEA, ChaCha20-Poly1305

Paper: https://www.ncbi.nlm.nih.gov/pmc/articles/PMC6806263/#sec6-sensors-19-04312

Considerations:
 * block cypher vs stream cypher.
 * target small ciphertext.  (maybe not relevant?)
 * should measure size of ciphertext vs GenerationAddress ciphertext in
   PublicAnnouncement.
 * must be quantum-secure.
 * we want to derive from our existing wallet seed + index/nonce.
 * is it a crazy idea to use one-time pad?  how large are our plaintext secrets?

## To Derive, or not to Derive

We have a choice whether to use a single symmetric key that all
`SymmetricKey` notifications would be encrypted to, or to derive a new symmetric
key for each UTXO.

Using a single key has the drawback that if the key is ever stolen or published,
then all UTXO notifications encrypted to it can be decrypted revealing the inner
UTXO of each.  Moreover, we require a mechanism such as the `PublicAnnouncement`
`receiver_id` in order to match transaction outputs with our key. For a single
key, the derived and publicly visible `receiver_id` would always be the same,
which links these UTXOs together -- making this option unviable.

Using derived keys, each key only unlocks a single UTXO, so loss of a
single key does not enable UTXO linking.  As such, key derivation seems the path
we should take, ceteris paribus.

A consequence of using key derivation is that we may incur additional cost
when scanning for utxos that our wallet can claim.  It is important to find an
efficient construction.


## Key Derivation

The symmetric keys will be derived from the same wallet seed as the public keys.
This is necessary so that Alice can restore all funds in her wallet, including
change addresses, using only her seed mnemonic.

There already exists

```
WalletState::nth_generation_spending_key(&self, counter: u16)
```

Thus we could add:

```
fn WalletState::nth_symmetric_key(&self, counter: u16)
```

### 2^16 limitation

It is worth discussing the `counter: u16`.  Presently the code has a comment:

```
// We keep n between 0 and 2^16 as this makes it possible to scan all possible
// addresses in case you don't know with what counter you made the address
```

This limits the number of derived keys to 65536, which is not really that much, especially if one considers:
 * high volume merchants
 * exchanges
 * high frequency traders
 * wallet consolidation transactions
 * not all generated keys actually receive funds

Further, it seems a hidden privacy bug as the counter will simply wrap around to 0,which means that keys can/will be re-used, perhaps over and over.

I think we should re-consider this (mis?)-feature.  If we do decide to keep it, then at minimum we should carefully document it for end-users, as I would consider it "surprising behavior" that I'm unaware of in other cryptocurrencies.

### Index/Counter

The `WalletState` would have a new field, `next_symmetric_key_index` that would be persisted in the wallet DB.  This field represents the index of the next symmetric key to use.  When the wallet is created, the index will be 0.

We will have a wallet method for generating a new symmetric key that increments this counter, eg:

```
fn next_symmetric_key(&mut self) -> SymmetricKey {
    let key = SymmetricKey::from_seed_and_index(self.seed, self.next_symmetric_key_index);
    self.next_symmetric_key_index += 1;
    key
}
```

### Announce & Claim Mechanism

Claiming transaction outputs involves matching each output UTXO in a block to
any of the possible addresses in our wallet. Thus the unoptimized operation
potentially involves m block outputs times n possible addresses, where n =
2^512 [1]. In practice, the number of addresses is much smaller because the wallet
knows how many addresses it has issued, and thus places a ceiling on n. Even
still, one can see that if m and n are both 1000+, this becomes an expensive
operation: O(n*m)

We do not necessarily have to optimize this operation for a first
implementation, however it is clear that we will need to do so eventually
if/when blocks begin to fill up and people are using wallets with many
transactions.

We have an existing mechanism for announcing and claiming UTXOs called
`PublicAnnouncement`.  The claiming mechanism presently works in the naive way,
checking each address against each transaction output to find matches and is
thus O(m*n).

Alternatively, Monero has a sub-address scheme that is O(n) with the number of
transaction outputs.  I believe we can adapt our claim mechanism to use their
optimization.

Both mechanisms are reviewed below.

#### Neptune PublicAnnouncement

##### Sending: Creating a UTXO Announcement

For each output UTXO in a transaction:

* `ReceivingAddress::generate_public_announcement()` is called, which:
    * encrypts (`utxo`, `sender_randomness`) with the address's public-key.
    * returns a `PublicAnnouncement(vec![GENERATION_FLAG, address.receiver_identifier, ciphertext])`

note: `GENERATION_FLAG` and `receiver_identifier` are both unencrypted (on the blockchain).

note: `GENERATION_FLAG = BFieldElement(79)`

note: The `receiver_id` is derived from the `ReceivingAddress` seed/digest as:

```
fn derive_receiver_id(seed: Digest) -> BFieldElement {
    Hash::hash_varlen(
        &[seed.values().to_vec(),
        vec![BFieldElement::new(2)]].concat()
    ).values()[0]
}
```

note: a `PublicAnnouncement` is simply a list of `BFieldElement`:

```
pub struct PublicAnnouncement {
    pub message: Vec<BFieldElement>,
}
```

##### Receiving: Claiming a UTXO

`WalletState::scan_for_announced_utxos()` loops over every known address in our wallet:

* `SpendingKey::scan_for_announced_utxos()` loops over all transaction public_announcements and:
    * checks if the announcement is marked as a GenerationAddress via GENERATION_FLAG.
    * retrieves a `receiver_id` from the public announcement, if present.
    * checks if the `receiver_id` matches `SpendingKey::receiver_identifier`.

* For any matches, the announcement ciphertext is then decrypted with the
`SpendingKey`, yielding the utxo and sender_randomness.

note: The `SpendingKey` and `ReceivingAddress` are both instantiated with the
same seed/digest and both use same `derive_receiver_id()` to derive the same
`receiver_identifer`.

##### Adapting for symmetric keys

It seems straight-forward to adapt this mechanism to symmetric keys.

We would have some kind of `SymmetricAddress` type with a `receiver_identifier`
field which is derived from the seed/Digest of the key.

When sending, we would use a SYMMETRIC_FLAG in place of the GENERATION_FLAG to
generate a `PublicAnnouncement` and would encrypt to our symmetric key.

When claiming, we would use the same algorithm as GenerationAddress, but would
check for SYMMETRIC_FLAG instead of GENERATION_FLAG, and would decrypt with our
symmetric key.

###### Minor improvement: single loop over outputs

It may be a bit more efficient to perform a single loop over the transaction
outputs and check if each has a GENERATION_FLAG or SYMMETRIC_FLAG.

The present claiming algo is basically:

```
secrets = vec![]
for spending_key in SpendingKeys:
    for (type, receiver_id, ciphertext) in transaction.public_announcements:
        if type == GENERATION_FLAG && receiver_id == spending_key.receiver_id:
            secrets.push(spending_key.decrypt(ciphertext))
```

If we duplicated this for symmetric keys, we would have:

```
secrets = vec![]
for spending_key in SpendingKeys:
    for (type, receiver_id, ciphertext) in transaction.public_announcements:
        if type == GENERATION_FLAG && receiver_id == spending_key.receiver_id:
            secrets.push(spending_key.decrypt(ciphertext))

for symmetric_key in SymmetricKeys:
    for (type, receiver_id, ciphertext) in transaction.public_announcements:
        if type == SYMMETRIC_FLAG && receiver_id == spending_key.receiver_id:
            secrets.push(symmetric_key.decrypt(ciphertext))
```

We iterate over transaction.public_announcements twice.  This is O(n*(y+z)).

Instead, we could do:

```
secrets = vec![]
for (type, receiver_id, ciphertext) in transaction.public_announcements:
    match type:
        GENERATION_FLAG:
            for spending_key in SpendingKeys:
                if spending_key.receiver_id == receiver_id:
                    secrets.push(spending_key.decrypt(ciphertext))

        SYMMETRIC_FLAG:
            for symmetric_key in SpendingKeys:
                if symmetric_key.receiver_id == receiver_id:
                    secrets.push(symmetric_key.decrypt(ciphertext))
```

We iterate over transaction.public_announcements once.  This is still O(n*(y+z))
complexity but with a smaller constant actually iterating over n.

Is it worth doing?  By itself that's debateable.  It would entail a bit of
refactoring but should not be a big deal.  However it can be used with a bigger
optimization detailed below.


#### Monero Sub-Addresses use pre-computed hash table.

Monero uses a key-derivation scheme called sub-addresses that is able to
determine if a given utxo is owned by the wallet in constant time.

It does this by pre-computing a large number (default: 10,000) of sub-addresses
and storing derived secrets in a hash-table where `hash(secret) --> index`.

The incoming utxo has a pubkey which is included in a calculation such that the
resulting value should match a secret stored in the hash table.  If it does,
then the wallet can claim/spend the UTXO.

Quote:

> Bob checks if an output pubkey P in a new transaction belongs to him or not by
> computing `D' = P - Hs(a*R)*G` and looking for `D'` in the hash table

Thus, this is an O(1) operation for a given UTXO, or O(n) when checking n UTXOs.

Full details of the scheme can be found at:

* PR: https://github.com/monero-project/monero/pull/2056
* Paper: https://web.getmonero.org/resources/research-lab/pubs/MRL-0006.pdf



#### Improvement: combine the two approaches

The `PublicAnnouncement` mechanism already provides a way to check if a given
address matches a given UTXO via the `recipient_id` which is non-reversibly
derived from the `ReceivingAddress` seed.

Given that, it seems we can use Monero's trick of pre-computing a large hash
table where `hash(recipient_id) --> index`.

Our claim loop can then get rid of the inner loop, eg:

```
let secrets = vec![]
for (type, recipient_id, ciphertext) in transaction.public_announcements:
    let secret = match type:
        GENERATION_FLAG:
            if let Some(index) = spending_key_table.get(recipient_id):
                let spending_key = spending_seed.derive(index)
                spending_key.decrypt(ciphertext)
        SYMMETRIC_FLAG:
            if let Some(index) = symmetric_key_table.get(recipient_id):
                let symmetric_key = symmetric_seed.derive(index)
                symmetric_key.decrypt(ciphertext)
    secrets.push(secret)
```

Notes:
 1. The hash-table is stored on disk and is growable if the 10,000 should be
    exceeded.  There is no limit to the number of sub-addresses.

 2. The hash table could directly store the key rather than a derivation index.
 This eliminates the call to derive() in the claim loop but requires more RAM
 and disk space.  For symmetric keys this *might* be an acceptable tradeoff,
 however for asymmetric keys which are quite large, that seems doubtful.

 3. This improves claim efficiency for our GenerationAddress as well, which
    has the same naive complexity of O(n*m) for claiming.


Is it worth doing?  I think so.

Can it be done later?   Probably, but if we do it now we can determine if there
are any hidden gotchas.




[1] twenty_first::math::lattice::kem::SecretKey::key is defined as [u8; 32]
    ie: 256 bits.
