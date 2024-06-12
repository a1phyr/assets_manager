use std::{any::TypeId, hash::BuildHasher};

use crate::{entry::CacheEntry, UntypedHandle};
use hashbrown::HashTable;

pub(crate) struct AssetMap {
    map: HashTable<CacheEntry>,
}

impl AssetMap {
    pub fn new() -> AssetMap {
        AssetMap {
            map: HashTable::new(),
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
            .entry(hash, |e| e.as_key() == key, |e| hasher.hash_one(e))
            .or_insert(entry);

        entry.into_mut().inner()
    }

    pub fn take(&mut self, hash: u64, id: &str, type_id: TypeId) -> Option<CacheEntry> {
        self.map
            .find_entry(hash, |e| e.as_key() == (type_id, id))
            .ok()
            .map(|e| e.remove().0)
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn iter_for_debug(&self) -> impl Iterator<Item = (&str, &CacheEntry)> + '_ {
        self.map.iter().map(|e| (e.as_key().1, e))
    }
}
