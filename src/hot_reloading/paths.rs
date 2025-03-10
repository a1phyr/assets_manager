use crate::{
    AnyCache, SharedString,
    cache::AssetMap,
    source::{OwnedDirEntry, Source},
    utils::{HashSet, OwnedKey},
};

use super::{dependencies::DepsGraph, records::Dependencies};

#[derive(Clone, Copy)]
struct BorrowedCache<'a> {
    assets: &'a AssetMap,
    source: &'a (dyn Source + 'static),
    reloader: &'a super::HotReloader,
}

impl<'a> crate::anycache::RawCache for BorrowedCache<'a> {
    type AssetMap = AssetMap;
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

pub(crate) struct AssetReloadInfos(OwnedKey, Dependencies, crate::key::Type);

impl AssetReloadInfos {
    #[inline]
    pub(crate) fn from_type(id: SharedString, deps: Dependencies, typ: crate::key::Type) -> Self {
        let key = OwnedKey::new_with(id, typ.type_id);
        Self(key, deps, typ)
    }
}

enum CacheKind {
    Local,
    Static(&'static AssetMap, &'static super::HotReloader),
}

pub(super) struct HotReloadingData {
    source: Box<dyn Source>,
    to_reload: HashSet<OwnedDirEntry>,
    cache: CacheKind,
    deps: DepsGraph,
}

impl HotReloadingData {
    pub fn new(source: Box<dyn Source>) -> Self {
        HotReloadingData {
            source,
            to_reload: HashSet::new(),
            cache: CacheKind::Local,
            deps: DepsGraph::new(),
        }
    }

    pub fn handle_events(&mut self, events: super::Events) {
        events.for_each(|entry| {
            if self.deps.contains(&entry) {
                log::trace!("New event: {entry:?}");
                self.to_reload.insert(entry);
            }
        });
        self.update_if_static();
    }

    pub fn update_if_local(&mut self, cache: &AssetMap, reloader: &super::HotReloader) {
        if let CacheKind::Local = &mut self.cache {
            let cache = BorrowedCache::new(cache, reloader, &self.source);
            run_update(&mut self.to_reload, &mut self.deps, cache);
        }
    }

    fn update_if_static(&mut self) {
        if let CacheKind::Static(cache, reloader) = &mut self.cache {
            let cache = BorrowedCache::new(cache, reloader, &self.source);
            run_update(&mut self.to_reload, &mut self.deps, cache);
        }
    }

    /// Drop the local cache and use the static reference we have on the
    /// `AssetCache`.
    pub fn use_static_ref(
        &mut self,
        asset_cache: &'static AssetMap,
        reloader: &'static super::HotReloader,
    ) {
        if let CacheKind::Local = &mut self.cache {
            self.cache = CacheKind::Static(asset_cache, reloader);
            log::trace!("Hot-reloading now use a 'static reference");

            let cache = BorrowedCache::new(asset_cache, reloader, &self.source);
            run_update(&mut self.to_reload, &mut self.deps, cache);
        }
    }

    pub fn add_asset(&mut self, infos: AssetReloadInfos) {
        let AssetReloadInfos(key, new_deps, typ) = infos;
        self.deps.insert_asset(key, new_deps, typ);
    }

    pub fn clear_local_cache(&mut self) {
        self.to_reload.clear();
    }
}

fn run_update(changed: &mut HashSet<OwnedDirEntry>, deps: &mut DepsGraph, cache: BorrowedCache) {
    let to_update = deps.topological_sort_from(changed.iter());
    changed.clear();

    for key in to_update.into_iter() {
        deps.reload(cache.as_any_cache(), key);
    }
}
