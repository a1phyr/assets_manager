use super::{Dependencies, ReloadFn};
use crate::utils::{HashMap, HashSet, OwnedKey};
use std::collections::hash_map::Entry;

struct AssetDeps {
    reload: Option<ReloadFn>,
    rdeps: HashSet<OwnedKey>,
    deps: Dependencies,
}

impl Default for AssetDeps {
    fn default() -> Self {
        AssetDeps {
            reload: None,
            deps: Dependencies::empty(),
            rdeps: HashSet::new(),
        }
    }
}

impl AssetDeps {
    fn new(reload: Option<ReloadFn>, deps: Dependencies) -> Self {
        AssetDeps {
            reload,
            deps,
            rdeps: HashSet::new(),
        }
    }
}

pub(crate) struct DepsGraph(HashMap<OwnedKey, AssetDeps>);

impl DepsGraph {
    pub fn new() -> Self {
        DepsGraph(HashMap::new())
    }

    pub fn insert(&mut self, asset_key: OwnedKey, deps: Dependencies, reload: Option<ReloadFn>) {
        for key in deps.iter() {
            let entry = self.0.entry(key.clone()).or_insert_with(AssetDeps::default);
            entry.rdeps.insert(asset_key.clone());
        }

        match self.0.entry(asset_key.clone()) {
            Entry::Vacant(e) => {
                let entry = AssetDeps::new(reload, deps);
                e.insert(entry);
            }
            Entry::Occupied(e) => {
                let entry = e.into_mut();
                let removed: Vec<_> = entry.deps.difference(&deps).cloned().collect();
                entry.deps = deps;
                entry.reload = reload;

                for key in removed {
                    // The None case is not supposed to happen, but we can safely
                    // ignore it
                    if let Some(entry) = self.0.get_mut(&key) {
                        entry.rdeps.remove(&asset_key);
                    }
                }
            }
        }
    }
}

struct TopologicalSortData {
    visited: HashSet<OwnedKey>,
    list: Vec<OwnedKey>,
}

fn visit(dep_graph: &DepsGraph, sort: &mut TopologicalSortData, key: &OwnedKey, add_self: bool) {
    if sort.visited.contains(key) {
        return;
    }

    let deps = match dep_graph.0.get(key) {
        Some(deps) => deps,
        None => return,
    };

    for rdep in deps.rdeps.iter() {
        visit(dep_graph, sort, rdep, true);
    }

    sort.visited.insert(key.clone());
    if add_self {
        sort.list.push(key.clone());
    }
}

pub(crate) struct AssetDepGraph(Vec<OwnedKey>);

impl AssetDepGraph {
    pub fn new<'a, I: IntoIterator<Item = &'a OwnedKey>>(dep_graph: &DepsGraph, iter: I) -> Self {
        let mut sort = TopologicalSortData {
            visited: HashSet::new(),
            list: Vec::new(),
        };

        for key in iter {
            visit(dep_graph, &mut sort, key, false);
        }

        AssetDepGraph(sort.list)
    }

    pub fn update(&self, deps: &mut DepsGraph, cache: crate::AnyCache) {
        for key in self.0.iter().rev() {
            if let Some(entry) = deps.0.get_mut(key) {
                if let Some(reload) = entry.reload {
                    let new_deps = reload(cache, key.id.clone());

                    if let Some(new_deps) = new_deps {
                        deps.insert(key.clone(), new_deps, Some(reload));
                    }
                }
            }
        }
    }
}
