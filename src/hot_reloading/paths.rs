use crate::{
    key::AnyAsset,
    source::Source,
    utils::{BorrowedKey, HashMap, HashSet, OwnedKey},
    Compound, SharedString,
};

use super::{dependencies::Dependencies, DynAssetCache};

pub(crate) type ReloadFn = fn(cache: &DynAssetCache, id: &str) -> Option<HashSet<OwnedKey>>;

#[allow(clippy::redundant_closure)]
fn reload<T: Compound>(cache: &DynAssetCache, id: &str) -> Option<HashSet<OwnedKey>> {
    let key = BorrowedKey::new::<T>(id);
    let handle = cache.assets.get_entry(key)?.handle();

    match cache.record_load::<T>(id) {
        Ok((asset, deps)) => {
            handle.as_dynamic().write(asset);
            log::info!("Reloading \"{}\"", id);
            Some(deps)
        }
        Err(err) => {
            log::warn!("Error reloading \"{}\": {}", id, err);
            None
        }
    }
}

pub(crate) struct CompoundReloadInfos(OwnedKey, HashSet<OwnedKey>, ReloadFn);

impl CompoundReloadInfos {
    #[inline]
    pub(crate) fn of<A: Compound>(id: SharedString, deps: HashSet<OwnedKey>) -> Self {
        let key = OwnedKey::new::<A>(id);
        Self(key, deps, reload::<A>)
    }
}

/// Store assets until we can sync with the `AssetCache`.
pub struct LocalCache {
    changed: HashMap<OwnedKey, Box<dyn AnyAsset>>,
}

enum CacheKind {
    Local(LocalCache),
    Static(&'static DynAssetCache, Vec<OwnedKey>),
}

impl CacheKind {
    /// Reloads an asset
    ///
    /// `key.type_id == asset.type_id()`
    fn update(&mut self, key: OwnedKey, asset: Box<dyn AnyAsset>) {
        match self {
            CacheKind::Static(cache, to_reload) => {
                if let Some(entry) = cache.assets.get_entry(key.borrow()) {
                    asset.reload(entry);
                    log::info!("Reloading \"{}\"", key.id());
                }
                to_reload.push(key);
            }
            CacheKind::Local(cache) => {
                cache.changed.insert(key, asset);
            }
        }
    }
}

pub(super) struct HotReloadingData {
    source: Box<dyn Source>,
    cache: CacheKind,
    deps: Dependencies,
}

impl HotReloadingData {
    pub fn new(source: Box<dyn Source>) -> Self {
        let cache = LocalCache {
            changed: HashMap::new(),
        };

        HotReloadingData {
            source,
            cache: CacheKind::Local(cache),
            deps: Dependencies::new(),
        }
    }

    pub fn load_asset(&mut self, events: super::Events) {
        events.for_each(|key| match key.typ.load(&self.source, &key.id) {
            Ok(asset) => {
                self.cache.update(key.into_owned_key(), asset);
                self.update_if_static();
            }
            Err(err) => log::warn!("Error reloading \"{}\": {}", key.id, err.reason()),
        })
    }

    pub fn update_if_local(&mut self, cache: &DynAssetCache) {
        if let CacheKind::Local(local_cache) = &mut self.cache {
            local_cache.update(&mut self.deps, cache);
        }
    }

    fn update_if_static(&mut self) {
        if let CacheKind::Static(cache, to_reload) = &mut self.cache {
            let to_update = super::dependencies::AssetDepGraph::new(&self.deps, to_reload.iter());
            to_update.update(&mut self.deps, cache);
            to_reload.clear();
        }
    }

    /// Drop the local cache and use the static reference we have on the
    /// `AssetCache`.
    pub fn use_static_ref(&mut self, asset_cache: &'static DynAssetCache) {
        if let CacheKind::Local(cache) = &mut self.cache {
            cache.update(&mut self.deps, asset_cache);
            self.cache = CacheKind::Static(asset_cache, Vec::new());
            log::trace!("Hot-reloading now use a 'static reference");
        }
    }

    pub fn add_compound(&mut self, infos: CompoundReloadInfos) {
        let CompoundReloadInfos(key, new_deps, reload) = infos;
        self.deps.insert(key, new_deps, Some(reload));
    }

    pub fn clear_local_cache(&mut self) {
        if let CacheKind::Local(cache) = &mut self.cache {
            cache.changed.clear();
        }
    }
}

impl LocalCache {
    /// Update the `AssetCache` with data collected in the `LocalCache` since
    /// the last reload.
    fn update(&mut self, deps: &mut Dependencies, cache: &DynAssetCache) {
        let to_update =
            super::dependencies::AssetDepGraph::new(deps, self.changed.iter().map(|(k, _)| k));

        // Update assets
        for (key, value) in self.changed.drain() {
            log::info!("Reloading \"{}\"", key.id());

            cache.assets.update_or_insert(
                key,
                value,
                |value, entry| value.reload(entry.inner()),
                |value, id| value.create(id),
            );
        }

        to_update.update(deps, cache);
    }
}
