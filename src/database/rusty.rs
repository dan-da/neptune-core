use super::leveldb::LevelDB;
use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;
use tokio::task;
use twenty_first::leveldb::{
    batch::WriteBatch,
    iterator::Iterable,
    options::{Options, ReadOptions},
};
use twenty_first::leveldb_sys::Compression;
use twenty_first::storage::level_db::DB;

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

pub fn create_db_if_missing() -> Options {
    let mut opts = Options::new();
    opts.create_if_missing = true;
    opts
}

impl<Key, Value> LevelDB<Key, Value> for RustyLevelDB<Key, Value>
where
    Key: Serialize + DeserializeOwned,
    Value: Serialize + DeserializeOwned,
{
    /// Open or create a new or existing database
    fn new(db_path: &Path, options: &Options) -> Result<Self> {
        let database = DB::open(db_path, options)?;
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

    fn put(&self, key: Key, value: Value) {
        let key_bytes: Vec<u8> = bincode::serialize(&key).unwrap();
        let value_bytes: Vec<u8> = bincode::serialize(&value).unwrap();
        self.database.put(&key_bytes, &value_bytes).unwrap();
    }

    fn batch_write(&self, entries: impl IntoIterator<Item = (Key, Value)>) {
        let batch = WriteBatch::new();
        for (key, value) in entries.into_iter() {
            let key_bytes: Vec<u8> = bincode::serialize(&key).unwrap();
            let value_bytes: Vec<u8> = bincode::serialize(&value).unwrap();
            batch.put(&key_bytes, &value_bytes);
        }

        self.database.write(&batch, true).unwrap();
    }

    fn delete(&self, key: Key) -> Option<Value> {
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

/// `RustyLevelDbAsync` provides an async-friendly and clone-friendly wrapper
/// around [`RustyLevelDB`].
///
/// Methods in the underlying struct `LevelDB` from `rs-leveldb` crate are all sync
/// and they sometimes perfom blocking file IO.  It is discouraged to
/// call blocking IO from async code as it can lead to concurrency problems,
/// usually hidden until a certain level of load is reached.
///
/// The tokio page for spawn_blocking says:
///     In general, issuing a blocking call or performing a lot of compute in a
///     future without yielding is problematic, as it may prevent the executor from
///     driving other futures forward.
///
/// See:
///  * <https://github.com/Neptune-Crypto/neptune-core/issues/74>
///  * <https://internals.rust-lang.org/t/warning-when-calling-a-blocking-function-in-an-async-context/11440/5>
///  * <https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html>
///
/// Therefore `RustyLevelDbAsync` wraps the sync methods with `spawn_blocking()`
/// so that the tokio runtime can run the blocking IO on a thread where blocking
/// is acceptable
#[derive(Clone)]
pub struct RustyLevelDbAsync<Key, Value>(Arc<RustyLevelDB<Key, Value>>)
where
    Key: Serialize + DeserializeOwned,
    Value: Serialize + DeserializeOwned;

impl<Key, Value> core::fmt::Debug for RustyLevelDbAsync<Key, Value>
where
    Key: Serialize + DeserializeOwned,
    Value: Serialize + DeserializeOwned,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RustyLevelDBAsync").finish()
    }
}

impl<Key, Value> RustyLevelDbAsync<Key, Value>
where
    Key: Serialize + DeserializeOwned + Send + Sync + 'static,
    Value: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    /// Open or create a new or existing database asynchronously
    pub async fn new(db_path: &Path, options: &Options) -> Result<Self> {
        let options_async = OptionsAsync::from(options);
        let path = db_path.to_path_buf();

        let db =
            task::spawn_blocking(move || RustyLevelDB::new(&path, &options_async.into())).await??;

        Ok(Self(Arc::new(db)))
    }

    /// IMPORTANT:  the returne iterator is NOT async.  The database is queried
    /// synchrously so the caller will block.  Consider using
    /// `spawn_blocking()` task when using this iterator in async code.
    ///
    /// ALSO: this calls allocates all DB keys.  For large databases
    /// this could be problematic and is best to avoid.
    ///
    // todo: can we avoid allocating keys with collect()?
    // todo: can we create a true async iterator?
    // todo: perhaps refactor neptune, so it does not need/use a level-db iterator.
    pub fn iter(&self) -> Box<dyn Iterator<Item = (Key, Value)> + '_> {
        let inner = self.0.clone();
        let keys: Vec<_> = inner.database.keys_iter(&ReadOptions::new()).collect();

        Box::new(keys.into_iter().map(move |k| {
            let v = inner.database.get_u8(&k).unwrap().unwrap();

            (
                bincode::deserialize(&k).unwrap(),
                bincode::deserialize(&v).unwrap(),
            )
        }))
    }

    /// Get database value asynchronously
    pub async fn get(&self, key: Key) -> Option<Value> {
        let inner = self.0.clone();
        task::spawn_blocking(move || inner.get(key)).await.unwrap()
    }

    /// Set database value asynchronously
    pub async fn put(&self, key: Key, value: Value) {
        let inner = self.0.clone();
        task::spawn_blocking(move || inner.put(key, value))
            .await
            .unwrap()
    }

    /// Write database values as a batch asynchronously
    pub async fn batch_write(
        &self,
        entries: impl IntoIterator<Item = (Key, Value)> + Send + Sync + 'static,
    ) {
        let inner = self.0.clone();
        task::spawn_blocking(move || inner.batch_write(entries))
            .await
            .unwrap()
    }

    /// Delete database value asynchronously
    pub async fn delete(&self, key: Key) -> Option<Value> {
        let inner = self.0.clone();
        task::spawn_blocking(move || inner.delete(key))
            .await
            .unwrap()
    }
}

// We made this OptionsAsync struct because leveldb::options::Options cannot be
// passed between threads because it contains the `cache: Option<Cache>` field
// and Cache is not `Send`.  We can't do anything about that, so instead we
// send this OptionsAsync between threads, which does not have a Cache field.
//
// todo:  add a cache_size option specified in bytes.
struct OptionsAsync {
    pub create_if_missing: bool,
    pub error_if_exists: bool,
    pub paranoid_checks: bool,
    pub write_buffer_size: Option<usize>,
    pub max_open_files: Option<i32>,
    pub block_size: Option<usize>,
    pub block_restart_interval: Option<i32>,
    pub compression: Compression,
}
impl From<&Options> for OptionsAsync {
    fn from(o: &Options) -> Self {
        if o.cache.is_some() {
            panic!("cache option not supported for RustyLevelDbAsync");
        }

        Self {
            create_if_missing: o.create_if_missing,
            error_if_exists: o.error_if_exists,
            paranoid_checks: o.paranoid_checks,
            write_buffer_size: o.write_buffer_size,
            max_open_files: o.max_open_files,
            block_size: o.block_size,
            block_restart_interval: o.block_restart_interval,
            compression: o.compression,
        }
    }
}
impl From<Options> for OptionsAsync {
    fn from(o: Options) -> Self {
        Self::from(&o)
    }
}

impl From<&OptionsAsync> for Options {
    fn from(o: &OptionsAsync) -> Self {
        Self {
            create_if_missing: o.create_if_missing,
            error_if_exists: o.error_if_exists,
            paranoid_checks: o.paranoid_checks,
            write_buffer_size: o.write_buffer_size,
            max_open_files: o.max_open_files,
            block_size: o.block_size,
            block_restart_interval: o.block_restart_interval,
            compression: o.compression,
            cache: None,
        }
    }
}
impl From<OptionsAsync> for Options {
    fn from(o: OptionsAsync) -> Self {
        Self::from(&o)
    }
}
