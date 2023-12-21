use crate::util_types::sync::tokio as sync_tokio;
use std::sync::Arc;

// use rusty_leveldb::{WriteBatch, DB};
use twenty_first::{
    leveldb::batch::WriteBatch,
    shared_math::tip5::Digest,
    storage::level_db::DB,
    storage::storage_schema::{
        traits::*, DbtSchema, DbtSingleton, DbtVec, RustyKey, RustyReader, WriteOperation,
    },
    sync::AtomicRw,
};

use super::monitored_utxo::MonitoredUtxo;

pub(in super::super) struct RustyWalletDatabaseInner {
    // Holds references to monitored_utxos, sync_label, counter
    // so they can be all written to levelDB as a single atomic batch write.
    schema: DbtSchema<RustyReader>,

    // active_window_storage: DbtSingleton<RustyKey, RustyMSValue, Vec<u32>>,
    monitored_utxos: DbtVec<MonitoredUtxo>,

    // records which block the database is synced to
    sync_label: DbtSingleton<Digest>,

    // counts the number of output UTXOs generated by this wallet
    counter: DbtSingleton<u64>,
}

impl RustyWalletDatabaseInner {
    fn connect(db: DB) -> Self {
        let mut schema = DbtSchema::<RustyReader> {
            tables: AtomicRw::from(vec![]),
            reader: Arc::new(RustyReader { db }),
        };

        let monitored_utxos_storage = schema.new_vec::<MonitoredUtxo>("monitored_utxos");
        let sync_label_storage = schema.new_singleton::<Digest>(RustyKey("sync_label".into()));
        let counter_storage = schema.new_singleton::<u64>(RustyKey("counter".into()));

        Self {
            schema,
            monitored_utxos: monitored_utxos_storage,
            sync_label: sync_label_storage,
            counter: counter_storage,
        }
    }

    fn db(&self) -> &DB {
        &self.schema.reader.db
    }

    fn monitored_utxos(&self) -> DbtVec<MonitoredUtxo> {
        // note: this clone just increments an Arc reference.
        self.monitored_utxos.clone()
    }

    fn get_sync_label(&self) -> Digest {
        self.sync_label.get()
    }

    pub fn set_sync_label(&mut self, sync_label: Digest) {
        self.sync_label.set(sync_label);
    }

    fn get_counter(&self) -> u64 {
        self.counter.get()
    }

    fn set_counter(&mut self, counter: u64) {
        self.counter.set(counter);
    }
}

#[derive(Clone)]
pub struct RustyWalletDatabase {
    pub(in super::super) inner: sync_tokio::AtomicRw<RustyWalletDatabaseInner>,
}

impl RustyWalletDatabase {
    pub fn connect(db: DB) -> Self {
        let inner = RustyWalletDatabaseInner::connect(db);
        Self {
            inner: sync_tokio::AtomicRw::from(inner),
        }
    }

    pub async fn get_sync_label(&self) -> Digest {
        self.inner.lock(|r| r.get_sync_label()).await
    }

    pub async fn set_sync_label(&self, sync_label: Digest) {
        self.inner.lock_mut(|r| r.set_sync_label(sync_label)).await
    }

    pub async fn get_counter(&self) -> u64 {
        self.inner.lock(|r| r.get_counter()).await
    }

    pub async fn set_counter(&self, counter: u64) {
        self.inner.lock_mut(|r| r.set_counter(counter)).await
    }

    pub async fn monitored_utxos(&self) -> DbtVec<MonitoredUtxo> {
        // note: this clone just increments an Arc reference.
        self.inner.lock(|r| r.monitored_utxos()).await
    }

    pub async fn persist(&self) {
        self.inner.lock_mut(|r| r.persist()).await
    }

    pub async fn restore_or_new(&self) {
        self.inner.lock_mut(|r| r.restore_or_new()).await
    }
}

impl StorageWriter for RustyWalletDatabaseInner {
    /// Locking:
    ///  * acquires read lock for DbtSchema `tables`
    fn persist(&mut self) {
        let write_batch = WriteBatch::new();

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
        // future: evaluate sync vs async writes for wallet DB.
        self.db()
            .write(&write_batch, true)
            .expect("Pending operations should be written to DB");
    }

    /// Locking:
    ///  * acquires read lock for DbtSchema `tables`
    fn restore_or_new(&mut self) {
        self.schema.tables.lock(|tables| {
            for table in tables.iter() {
                table.restore_or_new();
            }
        });
    }
}
