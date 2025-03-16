use crate::{
    AnyCache, SharedString,
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
    Static(AnyCache<'static>),
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

    pub fn update_if_local(&mut self, cache: AnyCache) {
        if let CacheKind::Local = self.cache {
            run_update(&mut self.to_reload, &mut self.deps, cache);
        }
    }

    fn update_if_static(&mut self) {
        if let CacheKind::Static(cache) = self.cache {
            run_update(&mut self.to_reload, &mut self.deps, cache);
        }
    }

    /// Drop the local cache and use the static reference we have on the
    /// `AssetCache`.
    pub fn use_static_ref(&mut self, cache: AnyCache<'static>) {
        if let CacheKind::Local = self.cache {
            self.cache = CacheKind::Static(cache);
            log::trace!("Hot-reloading now use a 'static reference");

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

fn run_update(changed: &mut HashSet<OwnedDirEntry>, deps: &mut DepsGraph, cache: AnyCache) {
    let to_update = deps.topological_sort_from(changed.iter());
    changed.clear();

    for key in to_update.into_iter() {
        deps.reload(cache, key);
    }
}
