use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;
use twenty_first::leveldb::options::Options;

/// The interface we want for any LevelDB crate we may use.
pub trait LevelDB<Key, Value>
where
    Key: Serialize + DeserializeOwned,
    Value: Serialize + DeserializeOwned,
{
    fn new(db_path: &Path, options: Options) -> Result<Self>
    where
        Self: Sized;
    fn batch_write(&mut self, entries: &[(Key, Value)]);
    fn get(&self, key: Key) -> Option<Value>;
    fn put(&mut self, key: Key, value: Value);
    fn delete(&mut self, key: Key) -> Option<Value>;
}
