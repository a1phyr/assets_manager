//! This example shows the use of Compound assets: assets able to load other
//! assets, and their integration with hot-reloading.

use assets_manager::{loader, AnyCache, Asset, AssetCache, BoxedError, Compound, SharedString};
use serde::Deserialize;
use std::sync::Arc;

/// The monster that can be found in different levels
#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct Monster {
    name: String,
    description: String,
    health: u32,
}

/// Monsters are stored in RON
impl Asset for Monster {
    const EXTENSION: &'static str = "ron";
    type Loader = loader::RonLoader;
}

/// The format of a level description.
///
/// Compound assets should not do filesytem operations, so we do this with
/// another asset.
#[derive(Deserialize, Debug)]
struct LevelManifest {
    name: String,
    spawn_table: Vec<(String, u32)>,
}

impl Asset for LevelManifest {
    const EXTENSION: &'static str = "ron";
    type Loader = loader::RonLoader;
}

/// The structure we use to store an in-game level
#[derive(Debug)]
#[allow(dead_code)]
struct Level {
    id: SharedString,
    name: String,

    /// A list of (Monster, spawn chance)
    spawn_table: Vec<(Arc<Monster>, f32)>,
}

/// Specify how to load a Level.
///
/// It will load the the corresponding manifest, and the necessary monsters.
/// Note that when hot-reloading is enabled, `assets_manager` records the assets
/// a Compound depends on. When a dependency is reloading, the Coumpound is also
/// reloaded. You don't have to write hot-reloading-specific code.
impl Compound for Level {
    fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        // Load the manifest
        let raw_level = cache.load::<LevelManifest>(id)?.read();

        // Prepare the spawn table
        let mut spawn_table = Vec::with_capacity(raw_level.spawn_table.len());
        let total = raw_level.spawn_table.iter().map(|(_, n)| *n).sum::<u32>() as f32;

        // Load each monster and insert it in the spawn table
        for &(ref monster_id, spawn_chance) in &raw_level.spawn_table {
            let monster = cache.load::<Arc<Monster>>(monster_id)?.cloned();
            let spawn_chance = spawn_chance as f32 / total;

            spawn_table.push((monster, spawn_chance));
        }

        Ok(Level {
            id: id.clone(),
            name: raw_level.name.clone(),
            spawn_table,
        })
    }
}

fn main() -> Result<(), BoxedError> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    let cache = AssetCache::new("assets")?;

    // Load the Level from the cache
    let level = cache.load::<Level>("example.levels.forest")?;
    let mut watcher = level.reload_watcher();

    println!("{level:#?}");

    loop {
        cache.hot_reload();

        // Touching one of these files will cause `level` to be reloaded:
        //  - assets/example/levels/forest.ron
        //  - assets/example/monsters/goblin.ron
        //  - assets/example/monsters/giant_bat.ron
        if watcher.reloaded() {
            println!("{level:#?}");
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
