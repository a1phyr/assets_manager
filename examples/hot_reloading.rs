//! A demonstration of hot-reloading.
//!
//! In this example, the file `assets/example/hello.txt` is loaded as text.
//! It is automatically updated when this file is changed (you are of course
//! encouraged to try changing the value to see what happens).

use assets_manager::{AssetCache, BoxedError};
use std::{thread::sleep, time::Duration};

fn main() -> Result<(), BoxedError> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    let cache = AssetCache::new("assets")?;

    // The asset reference is obtained outside the loop
    let text = cache.load::<String>("example.hello")?;

    // Indefinitely print the content of the file. Hot-reloading is performed in
    // the background.
    loop {
        println!("{}", text.read());

        sleep(Duration::from_millis(200));
    }
}
