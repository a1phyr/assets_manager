use super::{Dependencies, ReloadFn};
use crate::utils::{HashMap, HashSet, OwnedKey};
use std::collections::hash_map::Entry;

struct GraphNode {
    /// `None` if the asset is part of the graph but we should not actually
    /// reload it when changed (eg when `load_owned` was used)
    reload: Option<ReloadFn>,

    /// Reverse dependencies (backward edges)
    rdeps: HashSet<OwnedKey>,

    /// Dependencies (forward edges)
    deps: Dependencies,
}

impl Default for GraphNode {
    fn default() -> Self {
        GraphNode {
            reload: None,
            deps: Dependencies::empty(),
            rdeps: HashSet::new(),
        }
    }
}

impl GraphNode {
    fn new(reload: Option<ReloadFn>, deps: Dependencies) -> Self {
        GraphNode {
            reload,
            deps,
            rdeps: HashSet::new(),
        }
    }
}

pub(crate) struct DepsGraph(HashMap<OwnedKey, GraphNode>);

impl DepsGraph {
    pub fn new() -> Self {
        DepsGraph(HashMap::new())
    }

    pub fn insert(&mut self, asset_key: OwnedKey, deps: Dependencies, reload: Option<ReloadFn>) {
        for key in deps.iter() {
            let entry = self.0.entry(key.clone()).or_default();
            entry.rdeps.insert(asset_key.clone());
        }

        match self.0.entry(asset_key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(GraphNode::new(reload, deps));
            }
            Entry::Occupied(entry) => {
                let entry = entry.into_mut();
                let removed: Vec<_> = entry.deps.difference(&deps).cloned().collect();
                entry.deps = deps;
                entry.reload = reload;

                for key in removed {
                    // The None case is not supposed to happen, but we can safely
                    // ignore it
                    if let Some(entry) = self.0.get_mut(&key) {
                        log::warn!("Inexistant reverse dependency");
                        entry.rdeps.remove(&asset_key);
                    }
                }
            }
        }
    }

    pub fn topological_sort_from<'a>(
        &self,
        iter: impl IntoIterator<Item = &'a OwnedKey>,
    ) -> TopologicalSort {
        let mut sort_data = TopologicalSortData {
            visited: HashSet::new(),
            list: Vec::new(),
        };

        for key in iter {
            self.visit(&mut sort_data, key, false);
        }

        TopologicalSort(sort_data.list)
    }

    fn visit(&self, sort_data: &mut TopologicalSortData, key: &OwnedKey, add_self: bool) {
        if sort_data.visited.contains(key) {
            return;
        }

        let node = match self.0.get(key) {
            Some(deps) => deps,
            None => return,
        };

        for rdep in node.rdeps.iter() {
            self.visit(sort_data, rdep, true);
        }

        sort_data.visited.insert(key.clone());
        if add_self {
            sort_data.list.push(key.clone());
        }
    }

    pub fn reload(&mut self, cache: crate::AnyCache, key: &OwnedKey) {
        if let Some(entry) = self.0.get_mut(key) {
            if let Some(reload) = entry.reload {
                let new_deps = reload(cache, key.id.clone());

                if let Some(new_deps) = new_deps {
                    self.insert(key.clone(), new_deps, Some(reload));
                }
            }
        }
    }
}

struct TopologicalSortData {
    visited: HashSet<OwnedKey>,
    list: Vec<OwnedKey>,
}

pub(crate) struct TopologicalSort(Vec<OwnedKey>);

impl TopologicalSort {
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &OwnedKey> {
        self.0.iter().rev()
    }
}
