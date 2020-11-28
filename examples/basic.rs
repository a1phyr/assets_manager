//! In this example, we define a custom asset type and load it from a cache.
//!
//! The asset is stored in RON, in the file `assets/example/monsters/goblin.ron`.

use assets_manager::{Asset, AssetCache, loader};
use serde::Deserialize;
use std::error::Error;


#[derive(Deserialize)]
struct Monster {
    name: String,
    description: String,
    health: u32,
}

impl Asset for Monster {
    // The extension used by our type
    const EXTENSION: &'static str = "ron";

    // The way we load our data: here we use RON format
    type Loader = loader::RonLoader;
}


fn main() -> Result<(), Box<dyn Error>> {
    // The cache used to load assets
    // Its root is directory `assets`
    let cache = AssetCache::new("assets")?;

    // Load an asset with type `Vec<MonsterStats>`
    // The result is a lock on the stats
    let goblin = cache.load::<Monster>("example.goblin")?;

    // Lock the asset for reading. This is necessary because we might want to
    // reload it from disk (eg with hot-reloading)
    let goblin = goblin.read();

    // Use it
    println!("A {} ({}) has {} HP", goblin.name, goblin.description, goblin.health);

    Ok(())
}
