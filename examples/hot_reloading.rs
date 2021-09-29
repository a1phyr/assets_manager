//! A demonstration of hot-reloading.
//!
//! In this example, the file `assets/example/hot.x` is loaded as a integer.
//! It is automatically updated when this file is changed (you are of course
//! encouraged to try changing the value to see what happens).

use assets_manager::{
    Asset, AssetCache,
    loader::{LoadFrom, ParseLoader},
};
use std::{error::Error, thread::sleep, time::Duration};


/// A simple `i32` wrapper
struct X(i32);

impl From<i32> for X {
    fn from(x: i32) -> X {
        X(x)
    }
}

impl Asset for X {
    const EXTENSION: &'static str = "x";

    // An asset of type X is loaded by parsing the file as an i32
    // X: From<i32> is needed for this
    type Loader = LoadFrom<i32, ParseLoader>;
}


fn main() -> Result<(), Box<dyn Error>> {
    let cache = AssetCache::new("assets")?;

    // The asset reference is obtained outside the loop
    let x = cache.load::<X>("example.hot")?;

    // Indefinitly reload assets if they changed and print `x`
    loop {
        cache.hot_reload();

        print!("{}\n", x.read().0);

        sleep(Duration::from_millis(200));
    }
}
