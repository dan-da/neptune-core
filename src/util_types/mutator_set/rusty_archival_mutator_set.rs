use std::sync::Arc;

use itertools::Itertools;

use twenty_first::{
    leveldb::batch::WriteBatch,
    shared_math::{bfield_codec::BFieldCodec, tip5::Digest},
    storage::level_db::DB,
    storage::storage_schema::{
        traits::*, DbtSchema, DbtSingleton, DbtVec, RustyKey, RustyValue, WriteOperation,
    },
    sync::AtomicRw,
    util_types::{algebraic_hasher::AlgebraicHasher, mmr::archival_mmr::ArchivalMmr},
};

use super::{
    active_window::ActiveWindow, archival_mutator_set::ArchivalMutatorSet, chunk::Chunk,
    mutator_set_kernel::MutatorSetKernel,
};

struct RamsReader {
    db: Arc<DB>,
}

impl StorageReader for RamsReader {
    fn get_many(&self, keys: &[RustyKey]) -> Vec<Option<RustyValue>> {
        keys.iter().cloned().map(|key| self.get(key)).collect_vec()
    }

    fn get(&self, key: RustyKey) -> Option<RustyValue> {
        self.db
            .get(&key.0)
            .expect("Should get value")
            .map(RustyValue)
    }
}

type AmsMmrStorage = DbtVec<Digest>;
type AmsChunkStorage = DbtVec<Chunk>;
pub struct RustyArchivalMutatorSet<H>
where
    H: AlgebraicHasher + BFieldCodec,
{
    pub ams: ArchivalMutatorSet<H, AmsMmrStorage, AmsChunkStorage>,
    schema: DbtSchema<RamsReader>,
    db: Arc<DB>,
    active_window_storage: DbtSingleton<Vec<u32>>,
    sync_label: DbtSingleton<Digest>,
}

impl<H: AlgebraicHasher + BFieldCodec> RustyArchivalMutatorSet<H> {
    pub fn connect(db: DB) -> RustyArchivalMutatorSet<H> {
        let db_pointer = Arc::new(db);
        let reader = RamsReader {
            db: db_pointer.clone(),
        };
        let reader_pointer = Arc::new(reader);
        let mut schema = DbtSchema::<RamsReader> {
            tables: AtomicRw::from(vec![]),
            reader: reader_pointer,
        };
        let aocl_storage = schema.new_vec::<Digest>("aocl");
        let swbf_inactive_storage = schema.new_vec::<Digest>("swbfi");
        let chunks = schema.new_vec::<Chunk>("chunks");
        let active_window_storage =
            schema.new_singleton::<Vec<u32>>(RustyKey("active_window".into()));
        let sync_label = schema.new_singleton::<Digest>(RustyKey("sync_label".into()));
        let active_window = ActiveWindow::<H>::new();
        let kernel = MutatorSetKernel::<H, ArchivalMmr<H, AmsMmrStorage>> {
            aocl: ArchivalMmr::<H, AmsMmrStorage>::new(aocl_storage),
            swbf_inactive: ArchivalMmr::<H, AmsMmrStorage>::new(swbf_inactive_storage),
            swbf_active: active_window,
        };
        let ams = ArchivalMutatorSet::<H, AmsMmrStorage, AmsChunkStorage> { chunks, kernel };
        Self {
            ams,
            schema,
            db: db_pointer,
            active_window_storage,
            sync_label,
        }
    }

    pub fn get_sync_label(&self) -> Digest {
        self.sync_label.get()
    }

    pub fn set_sync_label(&mut self, sync_label: Digest) {
        self.sync_label.set(sync_label);
    }
}

impl<H: AlgebraicHasher + BFieldCodec> StorageWriter for RustyArchivalMutatorSet<H> {
    fn persist(&mut self) {
        let write_batch = WriteBatch::new();

        self.active_window_storage
            .set(self.ams.kernel.swbf_active.sbf.clone());

        self.schema.tables.lock(|tables| {
            for table in tables.iter() {
                let operations = table.pull_queue();
                for op in operations {
                    match op {
                        WriteOperation::Write(key, value) => write_batch.put(&key.0, &value.0),
                        WriteOperation::Delete(key) => write_batch.delete(&key.0),
                    }
                }
            }
        });

        // Perform a syncronous write, to be on the safe side.
        // future: evaluate sync vs async writes for mutator set.
        self.db
            .write(&write_batch, true)
            .expect("Could not persist to database.");
    }

    fn restore_or_new(&mut self) {
        self.schema.tables.lock(|tables| {
            for table in tables.iter() {
                table.restore_or_new();
            }
        });

        // The field `digests` of ArchivalMMR should always have at
        // least one element (a dummy digest), owing to 1-indexation.
        self.ams.kernel.aocl.fix_dummy();
        self.ams.kernel.swbf_inactive.fix_dummy();

        // populate active window
        self.ams.kernel.swbf_active.sbf = self.active_window_storage.get();
    }
}

#[cfg(test)]
mod tests {
    use crate::util_types::mutator_set::mutator_set_trait::{commit, MutatorSet};
    use itertools::Itertools;
    use rand::{random, thread_rng, RngCore};
    use twenty_first::shared_math::tip5::Tip5;

    use crate::util_types::mutator_set::{
        ms_membership_proof::MsMembershipProof, shared::BATCH_SIZE,
    };
    use crate::util_types::test_shared::mutator_set::*;
    use twenty_first::util_types::mmr::mmr_trait::Mmr;

    use super::*;

    #[test]
    fn persist_test() {
        type H = Tip5;

        let num_additions = 150 + 2 * BATCH_SIZE as usize;
        let num_removals = 50usize;
        let mut rng = thread_rng();

        // let (mut archival_mutator_set, db) = empty_rustyleveldb_ams();
        let db = DB::open_new_test_database(false, None, None, None).unwrap();
        let db_path = db.path().clone();
        let mut rusty_mutator_set: RustyArchivalMutatorSet<H> =
            RustyArchivalMutatorSet::connect(db);
        println!("Connected to database");
        rusty_mutator_set.restore_or_new();
        println!("Restored or new odne.");

        let mut items = vec![];
        let mut mps = vec![];

        println!(
            "before additions mutator set contains {} elements",
            rusty_mutator_set.ams.kernel.aocl.count_leaves()
        );

        for _ in 0..num_additions {
            let (item, sender_randomness, receiver_preimage) = make_item_and_randomnesses();
            let addition_record =
                commit::<H>(item, sender_randomness, receiver_preimage.hash::<H>());
            let mp = rusty_mutator_set
                .ams
                .kernel
                .prove(item, sender_randomness, receiver_preimage);

            MsMembershipProof::batch_update_from_addition(
                &mut mps.iter_mut().collect_vec(),
                &items,
                &rusty_mutator_set.ams.kernel,
                &addition_record,
            )
            .expect("Cannot batch update from addition");

            mps.push(mp);
            items.push(item);
            rusty_mutator_set.ams.add(&addition_record);
        }

        println!(
            "after additions mutator set contains {} elements",
            rusty_mutator_set.ams.kernel.aocl.count_leaves()
        );

        // Verify membership
        for (mp, &item) in mps.iter().zip(items.iter()) {
            assert!(rusty_mutator_set.ams.verify(item, mp));
        }

        // Remove items
        let mut removed_items = vec![];
        let mut removed_mps = vec![];
        for _ in 0..num_removals {
            let index = rng.next_u64() as usize % items.len();
            let item = items[index];
            let membership_proof = mps[index].clone();
            let removal_record = rusty_mutator_set.ams.kernel.drop(item, &membership_proof);
            MsMembershipProof::batch_update_from_remove(
                &mut mps.iter_mut().collect_vec(),
                &removal_record,
            )
            .expect("Could not batch update membership proofs from remove");

            rusty_mutator_set.ams.remove(&removal_record);

            removed_items.push(items.remove(index));
            removed_mps.push(mps.remove(index));
        }

        // Let's store the active window back to the database and create
        // a new archival object from the databases it contains and then check
        // that this archival MS contains the same values
        let sync_label: Digest = random();
        rusty_mutator_set.set_sync_label(sync_label);

        println!(
            "at persistence mutator set aocl contains {} elements",
            rusty_mutator_set.ams.kernel.aocl.count_leaves()
        );

        // persist and drop
        rusty_mutator_set.persist();

        let active_window_before = rusty_mutator_set.ams.kernel.swbf_active.clone();

        drop(rusty_mutator_set); // Drop DB

        // new database
        let new_db = DB::open_test_database(&db_path, true, None, None, None)
            .expect("should open existing database");
        let mut new_rusty_mutator_set: RustyArchivalMutatorSet<H> =
            RustyArchivalMutatorSet::connect(new_db);
        new_rusty_mutator_set.restore_or_new();

        // Verify memberships
        println!(
            "restored mutator set contains {} elements",
            new_rusty_mutator_set.ams.kernel.aocl.count_leaves()
        );
        for (index, (mp, &item)) in mps.iter().zip(items.iter()).enumerate() {
            assert!(
                new_rusty_mutator_set.ams.verify(item, mp),
                "membership proof {index} does not verify"
            );
        }

        // Verify non-membership
        for (index, (mp, &item)) in removed_mps.iter().zip(removed_items.iter()).enumerate() {
            assert!(
                !new_rusty_mutator_set.ams.verify(item, mp),
                "membership proof of non-member {index} still valid"
            );
        }

        let retrieved_sync_label = new_rusty_mutator_set.get_sync_label();
        assert_eq!(sync_label, retrieved_sync_label);

        let active_window_after = new_rusty_mutator_set.ams.kernel.swbf_active.clone();

        assert_eq!(active_window_before, active_window_after);
    }
}
