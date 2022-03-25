use crate::{
    cache::AssetMap,
    key::AnyAsset,
    source::Source,
    utils::{HashMap, HashSet, OwnedKey},
    AnyCache, Compound, SharedString,
};

use super::dependencies::Dependencies;

pub(crate) type ReloadFn = fn(cache: AnyCache, id: &str) -> Option<HashSet<OwnedKey>>;

#[derive(Clone, Copy)]
struct BorrowedCache<'a> {
    assets: &'a AssetMap,
    source: &'a (dyn Source + 'static),
    reloader: &'a super::HotReloader,
}

impl<'a> crate::anycache::RawCache for BorrowedCache<'a> {
    fn assets(&self) -> &AssetMap {
        &self.assets
    }

    fn reloader(&self) -> Option<&super::HotReloader> {
        Some(&self.reloader)
    }
}

impl<'a> crate::anycache::RawCacheWithSource for BorrowedCache<'a> {
    type Source = &'a dyn Source;

    fn get_source(&self) -> &&'a (dyn Source + 'static) {
        &self.source
    }
}

impl<'a> BorrowedCache<'a> {
    fn new(
        assets: &'a AssetMap,
        reloader: &'a super::HotReloader,
        source: &'a (dyn Source + 'static),
    ) -> Self {
        Self {
            assets,
            reloader,
            source,
        }
    }

    fn as_any_cache(&self) -> AnyCache {
        crate::anycache::CacheWithSourceExt::_as_any_cache(self)
    }
}

#[allow(clippy::redundant_closure)]
fn reload<T: Compound>(cache: AnyCache, id: &str) -> Option<HashSet<OwnedKey>> {
    let handle = cache.get_cached::<T>(id)?;

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
    Static(
        &'static AssetMap,
        &'static super::HotReloader,
        Vec<OwnedKey>,
    ),
}

impl CacheKind {
    /// Reloads an asset
    ///
    /// `key.type_id == asset.type_id()`
    fn update(&mut self, key: OwnedKey, asset: Box<dyn AnyAsset>) {
        match self {
            CacheKind::Static(cache, _, to_reload) => {
                if let Some(entry) = cache.get_entry(key.borrow()) {
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

    pub fn update_if_local(&mut self, cache: &AssetMap, reloader: &super::HotReloader) {
        if let CacheKind::Local(local_cache) = &mut self.cache {
            let cache = BorrowedCache::new(cache, reloader, &self.source);
            local_cache.update(&mut self.deps, cache);
        }
    }

    fn update_if_static(&mut self) {
        if let CacheKind::Static(cache, reloader, to_reload) = &mut self.cache {
            let to_update = super::dependencies::AssetDepGraph::new(&self.deps, to_reload.iter());
            let cache = BorrowedCache::new(cache, reloader, &self.source);
            to_update.update(&mut self.deps, cache.as_any_cache());
            to_reload.clear();
        }
    }

    /// Drop the local cache and use the static reference we have on the
    /// `AssetCache`.
    pub fn use_static_ref(
        &mut self,
        asset_cache: &'static AssetMap,
        reloader: &'static super::HotReloader,
    ) {
        if let CacheKind::Local(local_cache) = &mut self.cache {
            let cache = BorrowedCache::new(asset_cache, reloader, &self.source);
            local_cache.update(&mut self.deps, cache);
            self.cache = CacheKind::Static(asset_cache, reloader, Vec::new());
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
    fn update(&mut self, deps: &mut Dependencies, cache: BorrowedCache) {
        let to_update =
            super::dependencies::AssetDepGraph::new(deps, self.changed.iter().map(|(k, _)| k));

        // Update assets
        for (key, value) in self.changed.drain() {
            log::info!("Reloading \"{}\"", key.id());
            cache.assets.update_or_insert(key, value);
        }

        to_update.update(deps, cache.as_any_cache());
    }
}
