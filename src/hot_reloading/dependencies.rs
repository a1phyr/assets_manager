use super::{BorrowedDependency, Dependencies, Dependency};
use crate::{
    key::Type,
    source::OwnedDirEntry,
    utils::{HashMap, HashSet, OwnedKey},
};
use hashbrown::hash_map::Entry;

struct GraphNode {
    /// `None` if the asset is part of the graph but we should not actually
    /// reload it when changed (eg when `load_owned` was used)
    typ: Option<Type>,

    /// Reverse dependencies (backward edges)
    rdeps: HashSet<Dependency>,

    /// Dependencies (forward edges)
    deps: Dependencies,
}

impl Default for GraphNode {
    fn default() -> Self {
        GraphNode {
            typ: None,
            deps: Dependencies::empty(),
            rdeps: HashSet::new(),
        }
    }
}

impl GraphNode {
    fn new(typ: Type, deps: Dependencies) -> Self {
        GraphNode {
            typ: Some(typ),
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

    pub fn insert_asset(&mut self, asset_key: OwnedKey, deps: Dependencies, typ: Type) {
        self.insert(Dependency::Asset(asset_key), deps, typ)
    }

    pub fn insert(&mut self, asset_key: Dependency, deps: Dependencies, typ: Type) {
        for key in deps.iter() {
            let entry = self.0.entry(key.clone()).or_default();
            entry.rdeps.insert(asset_key.clone());
        }

        match self.0.entry(asset_key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(GraphNode::new(typ, deps));
            }
            Entry::Occupied(entry) => {
                let entry = entry.into_mut();
                let removed: Vec<_> = entry.deps.difference(&deps).cloned().collect();
                entry.deps = deps;
                entry.typ = Some(typ);

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
        iter: impl IntoIterator<Item = &'a OwnedDirEntry>,
    ) -> TopologicalSort {
        let mut sort_data = TopologicalSortData {
            visited: HashSet::new(),
            list: Vec::new(),
        };

        for key in iter {
            self.visit(&mut sort_data, key.as_dependency());
        }

        TopologicalSort(sort_data.list)
    }

    fn visit(&self, sort_data: &mut TopologicalSortData, key: BorrowedDependency) {
        if sort_data.visited.contains(&key) {
            return;
        }

        let node = match self.0.get(&key) {
            Some(deps) => deps,
            None => return,
        };

        for rdep in node.rdeps.iter() {
            self.visit(sort_data, rdep.as_borrowed());
        }

        sort_data.visited.insert(key.into_owned());
        if let BorrowedDependency::Asset(key) = key {
            sort_data.list.push(key.clone());
        }
    }

    pub fn reload(&mut self, cache: crate::AnyCache, key: OwnedKey) {
        let id = &key.id;
        let b_key = BorrowedDependency::Asset(&key);
        if let Some(entry) = self.0.get_mut(&b_key) {
            if let Some(typ) = entry.typ {
                let new_deps = cache.reload_untyped(id.clone(), typ);

                if let Some(new_deps) = new_deps {
                    self.insert(Dependency::Asset(key), new_deps, typ);
                }
            }
        }
    }

    pub fn contains(&self, key: &OwnedDirEntry) -> bool {
        self.0.contains_key(&key.as_dependency())
    }
}

struct TopologicalSortData {
    visited: HashSet<Dependency>,
    list: Vec<OwnedKey>,
}

pub(crate) struct TopologicalSort(Vec<OwnedKey>);

impl TopologicalSort {
    pub fn into_iter(self) -> impl ExactSizeIterator<Item = OwnedKey> {
        self.0.into_iter().rev()
    }
}
