use super::{Dependencies, Dependency};
use crate::{
    cache::CacheId,
    key::AssetKey,
    utils::{HashMap, HashSet},
};
use hashbrown::{Equivalent, hash_map::Entry};

struct GraphNode {
    /// Reverse dependencies (backward edges)
    rdeps: HashSet<Dependency>,

    /// Dependencies (forward edges)
    deps: Dependencies,
}

impl Default for GraphNode {
    fn default() -> Self {
        GraphNode {
            deps: Dependencies::new(),
            rdeps: HashSet::new(),
        }
    }
}

impl GraphNode {
    fn new(deps: Dependencies) -> Self {
        GraphNode {
            deps,
            rdeps: HashSet::new(),
        }
    }
}

pub(crate) struct DepsGraph(HashMap<Dependency, GraphNode>);

impl DepsGraph {
    pub fn new() -> Self {
        DepsGraph(HashMap::new())
    }

    pub fn insert_asset(&mut self, asset_key: AssetKey, deps: Dependencies) {
        self.insert(Dependency::Asset(asset_key), deps)
    }

    pub fn insert(&mut self, asset_key: Dependency, deps: Dependencies) {
        for key in deps.iter() {
            let entry = self.0.entry(key.clone()).or_default();
            entry.rdeps.insert(asset_key.clone());
        }

        match self.0.entry(asset_key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(GraphNode::new(deps));
            }
            Entry::Occupied(entry) => {
                let entry = entry.into_mut();
                let removed: Vec<_> = entry.deps.difference(&deps).cloned().collect();
                entry.deps = deps;

                for key in removed {
                    let removed = match self.0.get_mut(&key) {
                        Some(entry) => entry.rdeps.remove(&asset_key),
                        None => false,
                    };
                    // This is not supposed to happen, so we log a warning,
                    // but we can safely ignore it
                    if !removed {
                        log::warn!("Inexistant reverse dependency");
                    }
                }
            }
        }
    }

    pub fn topological_sort_from<'a>(
        &self,
        iter: impl IntoIterator<Item = &'a Dependency>,
    ) -> TopologicalSort {
        let mut sort_data = TopologicalSortData {
            visited: HashSet::new(),
            list: Vec::new(),
        };

        for key in iter {
            self.visit(&mut sort_data, key);
        }

        TopologicalSort(sort_data.list)
    }

    fn visit(&self, sort_data: &mut TopologicalSortData, key: &Dependency) {
        if sort_data.visited.contains(key) {
            return;
        }

        let node = match self.0.get(key) {
            Some(deps) => deps,
            None => return,
        };

        for rdep in node.rdeps.iter() {
            self.visit(sort_data, rdep);
        }

        sort_data.visited.insert(key.clone());
        if let Dependency::Asset(key) = key {
            sort_data.list.push(key.clone());
        }
    }

    pub fn contains(&self, key: &Dependency) -> bool {
        self.0.contains_key(key)
    }

    pub fn remove_cache(&mut self, id: CacheId) {
        self.0.retain(|key, _| match key {
            Dependency::Asset(AssetKey { cache, .. }) => !id.equivalent(cache),
            _ => true,
        });
    }
}

struct TopologicalSortData {
    visited: HashSet<Dependency>,
    list: Vec<AssetKey>,
}

pub(crate) struct TopologicalSort(Vec<AssetKey>);

impl TopologicalSort {
    pub fn into_iter(self) -> impl ExactSizeIterator<Item = AssetKey> {
        self.0.into_iter().rev()
    }
}
