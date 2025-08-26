use std::{any::TypeId, fmt, hash::BuildHasher};

use crate::{
    UntypedHandle,
    entry::CacheEntry,
    utils::{RandomState, RwLock, RwLockReadGuard},
};

struct EntryMap {
    map: hashbrown::HashTable<CacheEntry>,
}

impl EntryMap {
    pub fn new() -> EntryMap {
        EntryMap {
            map: hashbrown::HashTable::new(),
        }
    }

    pub fn get(&self, hash: u64, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        let entry = self.map.find(hash, |e| e.as_key() == (type_id, id))?;
        Some(entry.inner())
    }

    pub fn insert(
        &mut self,
        hash: u64,
        entry: CacheEntry,
        hasher: &impl BuildHasher,
    ) -> &UntypedHandle {
        let key = entry.as_key();
        let entry = self
            .map
            .entry(hash, |e| e.as_key() == key, |e| hasher.hash_one(e.as_key()))
            .or_insert(entry);

        entry.into_mut().inner()
    }

    pub fn iter_for_debug(&self) -> impl Iterator<Item = (&str, &CacheEntry)> + '_ {
        self.map.iter().map(|e| (e.as_key().1, e))
    }
}

// Make shards go to different cache lines to reduce contention
#[repr(align(64))]
struct Shard(RwLock<EntryMap>);

/// A map to store assets, optimized for concurrency.
///
/// This type has several uses:
/// - Provide a safe wrapper to ensure that no issue with lifetimes happen.
/// - Make a sharded lock map to reduce contention on the `RwLock` that guard
///   inner `HashMap`s.
/// - Provide an interface with the minimum of generics to reduce compile times.
pub(crate) struct AssetMap {
    hash_builder: RandomState,
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

        let hash_builder = RandomState::default();
        let shards = (0..shards)
            .map(|_| Shard(RwLock::new(EntryMap::new())))
            .collect();

        AssetMap {
            hash_builder,
            shards,
        }
    }

    fn hash_one(&self, key: (TypeId, &str)) -> u64 {
        std::hash::BuildHasher::hash_one(&self.hash_builder, key)
    }

    fn get_shard(&self, hash: u64) -> &Shard {
        let id = (hash as usize) & (self.shards.len() - 1);
        &self.shards[id]
    }

    pub fn get(&self, id: &str, type_id: TypeId) -> Option<&UntypedHandle> {
        let hash = self.hash_one((type_id, id));
        let shard = self.get_shard(hash).0.read();
        let entry = shard.get(hash, id, type_id)?;
        unsafe { Some(entry.extend_lifetime()) }
    }

    pub fn insert(&self, entry: CacheEntry) -> &UntypedHandle {
        let hash = self.hash_one(entry.as_key());
        let shard = &mut *self.get_shard(hash).0.write();
        let entry = shard.insert(hash, entry, &self.hash_builder);
        unsafe { entry.extend_lifetime() }
    }

    pub fn iter_shards(&self) -> impl Iterator<Item = LockedShard<'_>> {
        self.shards.iter().map(|s| LockedShard(s.0.read()))
    }
}

impl fmt::Debug for AssetMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = f.debug_map();

        for shard in &*self.shards {
            map.entries(shard.0.read().iter_for_debug());
        }

        map.finish()
    }
}

pub(crate) struct LockedShard<'a>(RwLockReadGuard<'a, EntryMap>);

impl LockedShard<'_> {
    pub fn iter(&self) -> impl Iterator<Item = &UntypedHandle> {
        self.0.map.iter().map(|e| e.inner())
    }
}
