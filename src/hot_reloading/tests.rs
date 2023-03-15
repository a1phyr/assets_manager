use crate::{
    source::DirEntry,
    tests::{X, XS, Y, Z},
    AssetCache,
};
use std::{fs::File, io, io::Write, path::Path, sync::Arc};

fn sleep() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}

type Res = Result<(), Box<dyn std::error::Error>>;

fn write_i32(path: &Path, n: i32) -> io::Result<()> {
    let mut file = File::create(path)?;
    write!(file, "{n}")
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
            let _ = env_logger::try_init();

            let id = concat!("test.hot_asset.", $id);
            let cache = AssetCache::new("assets")?;

            test_scenario!(@leak cache $is_static);

            let path = cache.raw_source().path_of(DirEntry::File(id, "x"));
            write_i32(&path, $n)?;
            sleep();

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
    not_loaded: X,
}

#[test]
fn messages() {
    use super::*;
    use crate::utils::Mutex;

    struct MessageChecker(Mutex<Vec<UpdateMessage>>);

    impl UpdateSender for MessageChecker {
        fn send_update(&self, message: UpdateMessage) {
            match self.0.lock().pop() {
                Some(expected) => assert_eq!(message, expected),
                None => panic!("Unexpected message {message:?}"),
            }
        }
    }

    impl Drop for MessageChecker {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                assert!(self.0.lock().is_empty());
            }
        }
    }

    #[derive(Clone, Copy)]
    struct TestSource;

    impl crate::source::Source for TestSource {
        fn read(&self, _id: &str, _ext: &str) -> io::Result<crate::source::FileContent> {
            Ok(crate::source::FileContent::Slice(b"10"))
        }

        fn read_dir(&self, _id: &str, _f: &mut dyn FnMut(DirEntry)) -> io::Result<()> {
            Err(io::ErrorKind::NotFound.into())
        }

        fn exists(&self, entry: DirEntry) -> bool {
            entry.is_file()
        }

        fn make_source(&self) -> Option<Box<dyn crate::source::Source + Send>> {
            Some(Box::new(*self))
        }

        fn configure_hot_reloading(
            &self,
            _events: EventSender,
        ) -> Result<DynUpdateSender, crate::BoxedError> {
            let a_key = AssetKey::new::<X>("a".into());
            let b_key = AssetKey::new::<X>("b".into());

            // Expected events, in reverse order
            let events = vec![
                UpdateMessage::RemoveAsset(a_key.clone()),
                UpdateMessage::AddAsset(a_key.clone()),
                UpdateMessage::Clear,
                UpdateMessage::AddAsset(a_key.clone()),
                UpdateMessage::RemoveAsset(a_key.clone()),
                UpdateMessage::AddAsset(b_key),
                UpdateMessage::AddAsset(a_key),
            ];

            Ok(Box::new(MessageChecker(Mutex::new(events))))
        }
    }

    let mut cache = crate::AssetCache::with_source(TestSource);
    cache.load_expect::<X>("a");
    assert!(!cache.remove::<X>("b"));
    assert!(cache.take::<X>("b").is_none());
    cache.load_expect::<X>("b");
    assert!(cache.remove::<X>("a"));
    cache.load_expect::<X>("a");
    cache.clear();
    cache.load_expect::<X>("a");
    assert!(cache.take::<X>("a").is_some());

    // Make sure we don't send message for these ones
    cache.load_expect::<XS>("c");
    assert!(cache.remove::<XS>("c"));
}
