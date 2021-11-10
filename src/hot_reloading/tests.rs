use crate::{
    source::DirEntry,
    tests::{X, Y, Z},
    AssetCache,
};
use std::{fs::File, io, io::Write, path::Path, sync::Arc};

fn sleep() {
    std::thread::sleep(std::time::Duration::from_millis(100));
}

type Res = Result<(), Box<dyn std::error::Error>>;

fn write_i32(path: &Path, n: i32) -> io::Result<()> {
    let mut file = File::create(path)?;
    write!(file, "{}", n)
}

macro_rules! test_scenario {
    (@leak $cache:ident true) => { let $cache = Box::leak(Box::new($cache)); };
    (@leak $cache:ident false) => {};

    (@enhance $cache:ident true) => { $cache.enhance_hot_reloading(); };
    (@enhance $cache:ident false) => {};

    (@reload $cache:ident true) => {};
    (@reload $cache:ident false) => { $cache.hot_reload(); };

    (
        name: $name:ident,
        is_static: $is_static:tt,
        type: $load:ty,
        id: $id:literal,
        start_value: $n:literal,
        $(not_loaded: $not_loaded:ty,)?
    ) => {
        #[test]
        fn $name() -> Res {
            let id = concat!("test.hot_asset.", $id);
            let cache = AssetCache::new("assets")?;

            test_scenario!(@leak cache $is_static);

            let path = cache.source().path_of(DirEntry::File(id, "x"));
            write_i32(&path, $n)?;

            test_scenario!(@enhance cache $is_static);

            let asset = cache.load::<$load>(id)?;
            let mut watcher = asset.reload_watcher();
            assert_eq!(asset.read().0, $n);
            test_scenario!(@reload cache $is_static);
            assert!(!watcher.reloaded());

            let n = rand::random();
            write_i32(&path, n)?;
            sleep();
            test_scenario!(@reload cache $is_static);
            assert_eq!(asset.read().0, n);
            assert!(watcher.reloaded());
            assert!(!watcher.reloaded());
            $( assert!(!cache.contains::<$not_loaded>(id)); )?

            write_i32(&path, $n)?;
            sleep();
            test_scenario!(@reload cache $is_static);
            assert_eq!(asset.read().0, $n);
            assert!(watcher.reloaded());
            assert!(!watcher.reloaded());
            $( assert!(!cache.contains::<$not_loaded>(id)); )?

            Ok(())
        }
    };
}

test_scenario! {
    name: reload_asset,
    is_static: false,
    type: X,
    id: "a",
    start_value: 42,
}

test_scenario! {
    name: reload_asset_static,
    is_static: true,
    type: X,
    id: "b",
    start_value: 22,
}

test_scenario! {
    name: reload_compound,
    is_static: false,
    type: Y,
    id: "c",
    start_value: -7,
}

test_scenario! {
    name: reload_compound_static,
    is_static: true,
    type: Y,
    id: "d",
    start_value: 0,
}

test_scenario! {
    name: reload_compound_compound,
    is_static: false,
    type: Z,
    id: "e",
    start_value: 0,
}

test_scenario! {
    name: reload_arc_compound,
    is_static: false,
    type: Arc<Y>,
    id: "f",
    start_value: -5,
    not_loaded: Y,
}

test_scenario! {
    name: reload_arc_asset,
    is_static: true,
    type: Arc<X>,
    id: "g",
    start_value: 57,
}
