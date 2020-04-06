# Assets-manager

[![Crates.io](https://img.shields.io/crates/v/assets_manager.svg)](https://crates.io/crates/assets_manager)
[![Docs.rs](https://docs.rs/assets_manager/badge.svg)](https://docs.rs/assets_manager/)
![Minimum rustc version](https://img.shields.io/badge/rustc-1.42+-lightgray.svg)

Conveniently load, store and cache external resources.


It has multiple goals:
- Easy to use
- Light: Pay for what you take, no dependency bloat
- Fast: Share your resources between threads without using expensive `Arc::clone`

This crate follow semver convention and supports rustc 1.42.0 and higher.
Changing this is considered a breaking change.

**Note**: this crate is still under developpement and breaking changes will
happen in the future, but use, feedbacks and requests are welcome and encouraged.

## Example

Suppose that you have a file `assets/common/test.ron` containing this:

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
    const EXT: &'static str = "ron";

    // The serialization format
    type Loader = loader::RonLoader;
}


// Create a new cache to load assets under the "./assets" folder
let cache = AssetCache::new("assets");

// Get a lock on the asset
// This will load the file `./assets/common/test.ron`
let asset_lock = cache.load::<Point>("common.test")?;

// Lock the asset for reading
// Any number of read locks can exist at the same time,
// but none can exist when the asset is reloaded
let point = asset_lock.read();

// The asset is now ready to be used
assert_eq!(point.x, 5);
assert_eq!(point.y, -6);

// Loading the same asset retreives it from the cache
let other_lock = cache.load("common.test")?;
assert!(asset_lock.ptr_eq(&other_lock));
```

Hot-reloading is also very easy to use:

```rust
let cache = AssetCache::new("assets");
let asset_lock = cache.load::<Point>("common.test")?;

loop {
    println!("Current value: {:?}", asset_lock.read());
    cache.hot_reload();
}
```

## Features

Current features:
- Convenient load of external files
- Cache loaded assets
- Hot-reloading
- Built-in support of most common data formats with serde

Planned features:
- Load from different sources (archives, embeded)

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
