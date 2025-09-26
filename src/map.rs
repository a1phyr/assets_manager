use crate::{
    UntypedHandle,
    entry::CacheEntry,
    utils::{RandomState, RwLock, RwLockReadGuard},
};
use hashbrown::HashTable;
use std::{any::TypeId, fmt};

#[derive(Clone, Default)]
struct Hasher(RandomState);

impl Hasher {
    #[inline]
    fn hash_entry(&self, entry: &CacheEntry) -> u64 {
        let (type_id, id) = entry.as_key();
        self.hash_key(id, type_id)
    }

    fn hash_key(&self, id: &str, type_id: TypeId) -> u64 {
        use std::hash::*;

        // We use a custom implementation because we don't need the prefix-free
        // hash implementation of `str`, which saves us a hash call.
        let mut hasher = self.0.build_hasher();
        type_id.hash(&mut hasher);
        hasher.write(id.as_bytes());
        hasher.finish()
    }
}

// Make shards go to different cache lines to reduce contention
#[repr(align(64))]
struct Shard(RwLock<HashTable<CacheEntry>>);

/// A map to store assets, optimized for concurrency.
///
/// This type has several uses:
/// - Provide a safe wrapper to ensure that no issue with lifetimes happen.
/// - Make a sharded lock map to reduce contention on the `RwLock` that guard
///   inner `HashMap`s.
/// - Provide an interface with the minimum of generics to reduce compile times.
pub(crate) struct AssetMap {
    hasher: Hasher,
    shards: Box<[Shard]>,
}

impl AssetMap {
    pub fn new() -> AssetMap {
        let shards = match std::thread::available_parallelism() {
            Ok(n) => 4 * n.get().next_power_of_two(),
            Err(err) => {
                log::error!("Failed to get available parallelism: {err}");
                32
            }
        };

        let hasher = Hasher::default();
        let shards = (0..shards)
            .map(|_| Shard(RwLock::new(HashTable::new())))
            .collect();

        AssetMap { hasher, shards }
    }

    fn get_shard(&self, hash: u64) -> &Shard {
        let id = (hash as usize) & (self.shards.len() - 1);
        &self.shards[id]
    }

    pub fn get(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        let hash = self.hasher.hash_key(id, type_id);
        let shard = self.get_shard(hash).0.read();

        let entry = shard.find(hash, |e| e.as_key() == (type_id, id))?;

        unsafe { Some(entry.inner().extend_lifetime()) }
    }

    pub fn insert(&self, entry: CacheEntry) -> &UntypedHandle {
        let hash = self.hasher.hash_entry(&entry);
        let shard = &mut *self.get_shard(hash).0.write();

        let key = entry.as_key();
        let entry = shard
            .entry(hash, |e| e.as_key() == key, |e| self.hasher.hash_entry(e))
            .or_insert(entry)
            .into_mut();

        unsafe { entry.inner().extend_lifetime() }
    }

    pub fn iter_shards(&self) -> impl Iterator<Item = LockedShard<'_>> {
        self.shards.iter().map(|s| LockedShard(s.0.read()))
    }
}

impl fmt::Debug for AssetMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_list();

        for shard in &*self.shards {
            map.entries(shard.0.read().iter());
        }

        map.finish()
    }
}

pub(crate) struct LockedShard<'a>(RwLockReadGuard<'a, HashTable<CacheEntry>>);

impl LockedShard<'_> {
    pub fn iter(&self) -> impl Iterator<Item = &UntypedHandle> {
        self.0.iter().map(|e| e.inner())
    }
}
