//! In this example, we load a custom image format made for terminal.
//!
//! The image is encoded in Bincode format, and in the file `assets/example/demo.img`
//! (the current directory is supposed to be the root of the crate).


use assets_manager::{Asset, AssetCache, AssetError, loader};
use serde::Deserialize;
use std::{thread::sleep, time::Duration};

/// A tile of the image
#[derive(Debug, Deserialize)]
struct Tile {
    sprite: char,
    x: u32,
    y: u32,
}

/// The Rust representation of our custom image format
#[derive(Debug, Deserialize)]
struct Image {
    title: String,
    tiles: Vec<Tile>,
}

impl Image {
    /// Prints the image to the terminal
    fn print(&self) {
        // Activate dual screen and print title
        println!("\x1b[?1049h\x1b[3;13H{}", self.title);

        // Definitly not the best way to to do this
        for tile in &self.tiles {
            println!("\x1b[{};{}H{}", tile.x, tile.y, tile.sprite);
        }

        sleep(Duration::from_secs(2));

        // Desactivate dual screen
        println!("\x1b[?1049l");
    }
}

impl Asset for Image {
    // The extension used by our type
    const EXT: &'static str = "img";

    // The way we load the image
    type Loader = loader::BincodeLoader;
}

fn main() -> Result<(), AssetError> {
    // The cache used to load assets
    // Its root is directory `assets`
    let cache = AssetCache::new("assets");

    // Load an asset with type `Image`
    // The result is a lock on the image
    // This is necessary because we may want to reload it from disk
    let img_lock = cache.load::<Image>("example.demo")?;

    // Lock the image for reading
    let img = img_lock.read();

    // Finally, print the image
    img.print();

    Ok(())
}
