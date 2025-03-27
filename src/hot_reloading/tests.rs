use crate::{
    AssetCache, BoxedError, Compound, SharedString,
    source::{DirEntry, FileSystem},
    tests::{X, Y, Z},
};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
    sync::Arc,
};

fn sleep() {
    std::thread::sleep(std::time::Duration::from_millis(50));
}

type Res = Result<(), Box<dyn std::error::Error>>;

fn write_i32(path: &Path, n: i32) -> io::Result<()> {
    log::debug!("Write {n} at {path:?}");
    let mut file = File::create(path)?;
    write!(file, "{n}")
}

macro_rules! test_scenario {
    (
        name: $name:ident,
        type: $load:ty,
        id: $id:literal,
        start_value: $n:literal,
        $(not_loaded: $not_loaded:ty,)?
    ) => {
        #[test]
        fn $name() -> Res {
            let _ = env_logger::try_init();

            std::fs::create_dir_all("assets/test/hot_asset/")?;

            let id = concat!("test.hot_asset.", $id);
            let cache = AssetCache::new("assets")?;

            let source = cache.downcast_raw_source::<FileSystem>().unwrap();
            let path = source.path_of(DirEntry::File(id, "x"));
            write_i32(&path, $n)?;
            sleep();

            let asset = cache.load::<$load>(id)?;
            let mut watcher = asset.reload_watcher();
            assert_eq!(asset.read().0, $n);
            assert!(!watcher.reloaded());

            let n = rand::random();
            write_i32(&path, n)?;
            sleep();
            assert_eq!(asset.read().0, n);
            assert!(watcher.reloaded());
            assert!(!watcher.reloaded());
            $( assert!(!cache.contains::<$not_loaded>(id)); )?

            write_i32(&path, $n)?;
            sleep();
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
    type: X,
    id: "a",
    start_value: 42,
}

test_scenario! {
    name: reload_compound,
    type: Y,
    id: "c",
    start_value: -7,
}

test_scenario! {
    name: reload_compound_compound,
    type: Z,
    id: "e",
    start_value: 0,
}

test_scenario! {
    name: reload_arc_compound,
    type: Arc<Y>,
    id: "f",
    start_value: -5,
    not_loaded: Y,
}

test_scenario! {
    name: reload_arc_asset,
    type: Arc<X>,
    id: "g",
    start_value: 57,
    not_loaded: X,
}

#[test]
fn directory() -> Result<(), BoxedError> {
    let _ = env_logger::try_init();

    let _ = std::fs::remove_dir_all("assets/test/hot_dir/");
    std::fs::create_dir_all("assets/test/hot_dir/")?;
    write_i32("assets/test/hot_dir/a.x".as_ref(), 1)?;

    let cache = AssetCache::new("assets")?;

    let dir = cache.load_dir::<X>("test.hot_dir")?;
    let mut watcher = dir.reload_watcher();
    assert!(!watcher.reloaded());

    assert_eq!(dir.read().ids().collect::<Vec<_>>(), ["test.hot_dir.a"]);

    write_i32("assets/test/hot_dir/a.x".as_ref(), 1)?;
    sleep();
    assert!(!watcher.reloaded());

    write_i32("assets/test/hot_dir/b.x".as_ref(), 1)?;
    sleep();
    assert_eq!(
        dir.read().ids().collect::<Vec<_>>(),
        ["test.hot_dir.a", "test.hot_dir.b"]
    );

    assert!(watcher.reloaded());

    std::fs::remove_file("assets/test/hot_dir/b.x")?;
    sleep();
    assert_eq!(dir.read().ids().collect::<Vec<_>>(), ["test.hot_dir.a"]);
    assert!(watcher.reloaded());

    std::fs::remove_file("assets/test/hot_dir/a.x")?;
    sleep();
    assert_eq!(dir.read().ids().collect::<Vec<_>>().len(), 0);
    assert!(watcher.reloaded());

    Ok(())
}

#[test]
fn multi_threading() {
    let _ = env_logger::try_init();

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct MyAsset {
        a: i32,
        b: i32,
    }

    impl Compound for MyAsset {
        fn load(cache: &AssetCache, id: &SharedString) -> Result<Self, BoxedError> {
            let recorder = crate::hot_reloading::Recorder::current();

            let (a, b) = std::thread::scope(|s| {
                let a = s.spawn(|| recorder.install(|| cache.load_expect::<X>(&format!("{id}.a"))));
                let b = s.spawn(|| cache.load_expect::<X>(&format!("{id}.b")));

                (a.join().unwrap().read().0, b.join().unwrap().read().0)
            });

            Ok(MyAsset { a, b })
        }
    }

    fs::create_dir_all("assets/test/mt/").unwrap();
    write_i32("assets/test/mt/a.x".as_ref(), 1).unwrap();
    write_i32("assets/test/mt/b.x".as_ref(), 2).unwrap();

    let cache = AssetCache::new("assets").unwrap();

    let asset = cache.load_expect::<MyAsset>("test.mt");
    assert_eq!(asset.copied(), MyAsset { a: 1, b: 2 });

    write_i32("assets/test/mt/a.x".as_ref(), 3).unwrap();
    sleep();

    assert_eq!(asset.copied(), MyAsset { a: 3, b: 2 });

    write_i32("assets/test/mt/b.x".as_ref(), 4).unwrap();
    sleep();

    assert_eq!(asset.copied(), MyAsset { a: 3, b: 2 });
}
