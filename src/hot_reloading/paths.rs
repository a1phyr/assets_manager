use crate::{
    AssetCache, SharedString,
    source::OwnedDirEntry,
    utils::{HashSet, OwnedKey},
};

use super::{dependencies::DepsGraph, records::Dependencies};

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
    Static(&'static AssetCache),
}

pub(super) struct HotReloadingData {
    to_reload: HashSet<OwnedDirEntry>,
    cache: CacheKind,
    deps: DepsGraph,
}

impl HotReloadingData {
    pub fn new() -> Self {
        HotReloadingData {
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

    pub fn update_if_local(&mut self, asset_cache: &AssetCache) {
        if let CacheKind::Local = &mut self.cache {
            run_update(&mut self.to_reload, &mut self.deps, asset_cache);
        }
    }

    fn update_if_static(&mut self) {
        if let CacheKind::Static(asset_cache) = &mut self.cache {
            run_update(&mut self.to_reload, &mut self.deps, asset_cache);
        }
    }

    /// Drop the local cache and use the static reference we have on the
    /// `AssetCache`.
    pub fn use_static_ref(&mut self, asset_cache: &'static AssetCache) {
        if let CacheKind::Local = &mut self.cache {
            self.cache = CacheKind::Static(asset_cache);
            log::trace!("Hot-reloading now use a 'static reference");

            run_update(&mut self.to_reload, &mut self.deps, asset_cache);
        }
    }

    pub fn add_asset(&mut self, infos: AssetReloadInfos) {
        let AssetReloadInfos(key, new_deps, typ) = infos;
        self.deps.insert_asset(key, new_deps, typ);
    }
}

fn run_update(changed: &mut HashSet<OwnedDirEntry>, deps: &mut DepsGraph, cache: &AssetCache) {
    let to_update = deps.topological_sort_from(changed.iter());
    changed.clear();

    for key in to_update.into_iter() {
        deps.reload(cache.as_any_cache(), key);
    }
}
