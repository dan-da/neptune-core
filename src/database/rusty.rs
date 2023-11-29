use super::leveldb::LevelDB;
use anyhow::Result;
use twenty_first::leveldb::{
    batch::WriteBatch,
    iterator::Iterable,
    iterator::Iterator as DBIterator,
    options::{Options, ReadOptions},
};
use twenty_first::storage::level_db::DB;
// use rusty_leveldb::{DBIterator, LdbIterator, WriteBatch, DB};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::marker::PhantomData;
use std::path::Path;

pub struct RustyLevelDB<Key, Value>
where
    Key: Serialize + DeserializeOwned,
    Value: Serialize + DeserializeOwned,
{
    database: DB,
    _key: PhantomData<Key>,
    _value: PhantomData<Value>,
}

// We have to implement `Debug` for `RustyLevelDB` as the `State` struct
// contains a database object, and `State` is used as input argument
// to multiple functions where logging is enabled with the `instrument`
// attributes from the `tracing` crate, and this requires all input
// arguments to the function to implement the `Debug` trait as this
// info is written on all logging events.
impl<Key, Value> core::fmt::Debug for RustyLevelDB<Key, Value>
where
    Key: Serialize + DeserializeOwned,
    Value: Serialize + DeserializeOwned,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("").finish()
    }
}

pub fn default_options() -> Options {
    let mut opt = Options::new();
    opt.create_if_missing = true;
    opt
}

impl<Key, Value> LevelDB<Key, Value> for RustyLevelDB<Key, Value>
where
    Key: Serialize + DeserializeOwned,
    Value: Serialize + DeserializeOwned,
{
    /// Open or create a new or existing database
    fn new(db_path: &Path, options: Options) -> Result<Self> {
        let database = DB::open(db_path, &options)?;
        let database = Self {
            database,
            _key: PhantomData,
            _value: PhantomData,
        };
        Ok(database)
    }

    fn get(&self, key: Key) -> Option<Value> {
        let key_bytes: Vec<u8> = bincode::serialize(&key).unwrap();
        let value_bytes: Option<Vec<u8>> = self.database.get(&key_bytes).unwrap();
        value_bytes.map(|bytes| bincode::deserialize(&bytes).unwrap())
    }

    fn put(&mut self, key: Key, value: Value) {
        let key_bytes: Vec<u8> = bincode::serialize(&key).unwrap();
        let value_bytes: Vec<u8> = bincode::serialize(&value).unwrap();
        self.database.put(&key_bytes, &value_bytes).unwrap();
    }

    fn batch_write(&mut self, entries: &[(Key, Value)]) {
        let batch = WriteBatch::new();
        for (key, value) in entries.iter() {
            let key_bytes: Vec<u8> = bincode::serialize(key).unwrap();
            let value_bytes: Vec<u8> = bincode::serialize(value).unwrap();
            batch.put(&key_bytes, &value_bytes);
        }

        self.database.write(&batch, true).unwrap();
    }

    fn delete(&mut self, key: Key) -> Option<Value> {
        let key_bytes: Vec<u8> = bincode::serialize(&key).unwrap(); // add safety
        let value_bytes: Option<Vec<u8>> = self.database.get(&key_bytes).unwrap();
        let value_object = value_bytes.map(|bytes| bincode::deserialize(&bytes).unwrap());
        let status = self.database.delete(&key_bytes);

        match status {
            Ok(_) => value_object, // could be None, if record is not present
            Err(err) => panic!("database failure: {}", err),
        }
    }
}

impl<Key: Serialize + DeserializeOwned, Value: Serialize + DeserializeOwned>
    RustyLevelDB<Key, Value>
{
    pub fn new_iter(&self) -> RustyLevelDBIterator<Key, Value> {
        RustyLevelDBIterator::new(self)
    }

    // pub fn flush(&mut self) {
    //     self.database
    //         .flush()
    //         .expect("Database flushing to disk must succeed");
    // }
}

pub struct RustyLevelDBIterator<
    'a,
    Key: Serialize + DeserializeOwned,
    Value: Serialize + DeserializeOwned,
> {
    iterator: DBIterator<'a>,
    _key: PhantomData<Key>,
    _value: PhantomData<Value>,
}

impl<Key: Serialize + DeserializeOwned, Value: Serialize + DeserializeOwned> Iterator
    for RustyLevelDBIterator<'_, Key, Value>
{
    type Item = (Key, Value);

    fn next(&mut self) -> Option<Self::Item> {
        self.iterator.next().map(|(sk, sv)| {
            (
                bincode::deserialize(&sk).unwrap(),
                bincode::deserialize(&sv).unwrap(),
            )
        })
    }
}

impl<'a, Key: Serialize + DeserializeOwned, Value: Serialize + DeserializeOwned>
    RustyLevelDBIterator<'a, Key, Value>
{
    fn new(database: &'a RustyLevelDB<Key, Value>) -> Self {
        let iterator = database.database.iter(&ReadOptions::new());
        Self {
            iterator,
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}
