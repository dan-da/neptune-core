use std::sync::Arc;

// use rusty_leveldb::{WriteBatch, DB};
use twenty_first::{
    leveldb::batch::WriteBatch,
    shared_math::tip5::Digest,
    storage::level_db::DB,
    storage::storage_schema::{
        DbtSchema, DbtSingleton, DbtVec, RustyKey, RustyReader, RustyValue, StorageSingleton,
        StorageWriter, WriteOperation,
    },
};

use super::monitored_utxo::MonitoredUtxo;

pub struct RustyWalletDatabase {
    // pub ams: ArchivalMutatorSet<H, AmsMmrStorage, AmsChunkStorage>,
    // schema: DbtSchema<RustyKey, RustyMSValue, RamsReader>,
    schema: DbtSchema<RustyKey, RustyValue, RustyReader>,
    // db: Arc<DB>,

    // active_window_storage: Arc<Mutex<DbtSingleton<RustyKey, RustyMSValue, Vec<u32>>>>,
    pub monitored_utxos: DbtVec<RustyKey, RustyValue, u64, MonitoredUtxo>,

    // records which block the database is synced to
    sync_label: DbtSingleton<RustyKey, RustyValue, Digest>,

    // counts the number of output UTXOs generated by this wallet
    counter: DbtSingleton<RustyKey, RustyValue, u64>,
}

impl RustyWalletDatabase {
    pub fn connect(db: DB) -> Self {
        let mut schema = DbtSchema::<RustyKey, RustyValue, RustyReader> {
            tables: vec![],
            reader: Arc::new(RustyReader { db }),
        };

        let monitored_utxos_storage = schema.new_vec::<u64, MonitoredUtxo>("monitored_utxos");
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

    pub fn get_sync_label(&self) -> Digest {
        self.sync_label.get()
    }

    pub fn set_sync_label(&mut self, sync_label: Digest) {
        self.sync_label.set(sync_label);
    }

    pub fn get_counter(&self) -> u64 {
        self.counter.get()
    }

    pub fn set_counter(&mut self, counter: u64) {
        self.counter.set(counter);
    }
}

impl StorageWriter<RustyKey, RustyValue> for RustyWalletDatabase {
    fn persist(&mut self) {
        let write_batch = WriteBatch::new();

        for table in self.schema.tables.iter_mut() {
            let operations = table.pull_queue();
            for op in operations {
                match op {
                    WriteOperation::Write(key, value) => write_batch.put(&key.0, &value.0),
                    WriteOperation::Delete(key) => write_batch.delete(&key.0),
                }
            }
        }

        // Perform a syncronous write, to be on the safe side.
        // future: evaluate sync vs async writes for wallet DB.
        self.db()
            .write(&write_batch, true)
            .expect("Pending operations should be written to DB");
    }

    fn restore_or_new(&mut self) {
        for table in self.schema.tables.iter_mut() {
            table.restore_or_new();
        }
    }
}
