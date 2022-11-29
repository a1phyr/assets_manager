# Assets-manager

[![Crates.io](https://img.shields.io/crates/v/assets_manager.svg)](https://crates.io/crates/assets_manager)
[![Docs.rs](https://docs.rs/assets_manager/badge.svg)](https://docs.rs/assets_manager/)
![Minimum rustc version](https://img.shields.io/badge/rustc-1.61+-lightgray.svg)


This crate aims at providing a filesystem abstraction to easily load external resources.
It was originally thought for games, but can of course be used in other contexts.

Original idea was inspired by [Veloren](https://gitlab.com/veloren/veloren)'s assets system.


This crate follow semver convention and supports rustc 1.61 and higher.
Changing this is considered a breaking change.

## Goals

This crates focuses on:

- **Good performances**:\
  Crucial for perfomance-oriented applications such as games.\
  Loaded assets are cached so loading one several times is as fast as loading it once.
  This crate was thought for use with concurrency.

- **Hot-reloading**:\
  Hot-reloading means updating assets in memory as soon as the corresponding file is changed,
  without restarting your program. It may greatly ease development.\
  Your time is precious, and first-class support of hot-reloading helps you saving it.

- **Pleasant to use**:\
  A well-documented high-level API, easy to learn.\
  Built-in support of common formats: serialization, images, sounds.\
  Can load assets from a file system, a zip archive, or even embed them in a binary.

- **Lightness**:\
  Pay for what you take, no dependency bloat.

## Example

Suppose that you have a file `assets/common/position.ron` containing this:

```text
Point(
    x: 5,
    y: -6,
)
```

Then you can load it this way (with feature `ron` enabled):

```rust
use assets_manager::{Asset, AssetCache, loader};
use serde::Deserialize;

// The struct you want to load
#[derive(Deserialize)]
struct Point {
    x: i32,
    y: i32,
}

// Specify how you want the structure to be loaded
impl Asset for Point {
    // The extension of the files to look into
    const EXTENSION: &'static str = "ron";

    // The serialization format (RON)
    type Loader = loader::RonLoader;
}


// Create a new cache to load assets under the "./assets" folder
let cache = AssetCache::new("assets")?;

// Get a handle on the asset
// This will load the file `./assets/common/position.ron`
let handle = cache.load::<Point>("common.position")?;

// Lock the asset for reading
// Any number of read locks can exist at the same time,
// but none can exist when the asset is reloaded
let point = handle.read();

// The asset is now ready to be used
assert_eq!(point.x, 5);
assert_eq!(point.y, -6);

// Loading the same asset retreives it from the cache
let other_handle = cache.load("common.position")?;
assert!(other_handle.same_handle(&handle));
```

Hot-reloading is also very easy to use:

```rust
let cache = AssetCache::new("assets")?;
let handle = cache.load::<Point>("common.position")?;

loop {
    // Reload all cached files that changed
    cache.hot_reload();

    // Assets are updated without any further work
    println!("Current value: {:?}", handle.read());
}
```

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
