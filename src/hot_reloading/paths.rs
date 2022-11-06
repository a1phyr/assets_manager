use crate::{
    cache::AssetMap,
    key::AnyAsset,
    source::Source,
    utils::{HashMap, OwnedKey},
    AnyCache, SharedString,
};

use super::{dependencies::DepsGraph, records::Dependencies, ReloadFn};

#[derive(Clone, Copy)]
struct BorrowedCache<'a> {
    assets: &'a AssetMap,
    source: &'a (dyn Source + 'static),
    reloader: &'a super::HotReloader,
}

impl<'a> crate::anycache::RawCache for BorrowedCache<'a> {
    type Source = &'a dyn Source;

    fn assets(&self) -> &AssetMap {
        self.assets
    }

    fn get_source(&self) -> &&'a (dyn Source + 'static) {
        &self.source
    }

    fn reloader(&self) -> Option<&super::HotReloader> {
        Some(self.reloader)
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
        crate::anycache::CacheExt::_as_any_cache(self)
    }
}

pub(crate) struct CompoundReloadInfos(OwnedKey, Dependencies, ReloadFn);

impl CompoundReloadInfos {
    #[inline]
    pub(crate) fn from_type(
        id: SharedString,
        deps: Dependencies,
        typ: crate::key::Type,
        reload_fn: ReloadFn,
    ) -> Self {
        let key = OwnedKey::new_with(id, typ.type_id);
        Self(key, deps, reload_fn)
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
                if let Some(entry) = cache.get(&key.id, key.type_id) {
                    asset.reload(entry);
                    log::info!("Reloading \"{}\"", key.id);
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
    deps: DepsGraph,
}

impl HotReloadingData {
    pub fn new(source: Box<dyn Source>) -> Self {
        let cache = LocalCache {
            changed: HashMap::new(),
        };

        HotReloadingData {
            source,
            cache: CacheKind::Local(cache),
            deps: DepsGraph::new(),
        }
    }

    pub fn load_asset(&mut self, events: super::Events) {
        events.for_each(
            |key| match key.typ.load_from_source(&self.source, &key.id) {
                Ok(asset) => {
                    self.cache.update(key.into_owned_key(), asset);
                    self.update_if_static();
                }
                Err(err) => log::warn!("Error reloading \"{}\": {}", key.id, err.reason()),
            },
        )
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
    fn update(&mut self, deps: &mut DepsGraph, cache: BorrowedCache) {
        let to_update =
            super::dependencies::AssetDepGraph::new(deps, self.changed.iter().map(|(k, _)| k));

        // Update assets
        for (key, value) in self.changed.drain() {
            log::info!("Reloading \"{}\"", key.id);
            cache.assets.update_or_insert(key.id, key.type_id, value);
        }

        to_update.update(deps, cache.as_any_cache());
    }
}
