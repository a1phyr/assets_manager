use crate::{AssetCache, key::AssetKey, source::OwnedDirEntry, utils::HashSet};

use super::{dependencies::DepsGraph, records::Dependencies};

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

    pub fn update_if_local(&mut self, cache: &AssetCache) {
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
    pub fn use_static_ref(&mut self, cache: &'static AssetCache) {
        if let CacheKind::Local = self.cache {
            self.cache = CacheKind::Static(cache);
            log::trace!("Hot-reloading now use a 'static reference");

            run_update(&mut self.to_reload, &mut self.deps, cache);
        }
    }

    pub fn add_asset(&mut self, key: AssetKey, deps: Dependencies) {
        self.deps.insert_asset(key, deps);
    }
}

fn run_update(changed: &mut HashSet<OwnedDirEntry>, deps: &mut DepsGraph, cache: &AssetCache) {
    let to_update = deps.topological_sort_from(changed.iter());
    changed.clear();

    for key in to_update.into_iter() {
        let new_deps = cache.reload_untyped(&key.id, key.typ);

        if let Some(new_deps) = new_deps {
            deps.insert_asset(key, new_deps);
        };
    }
}
