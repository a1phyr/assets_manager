use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X(pub i32);

impl FileAsset for X {
    const EXTENSION: &'static str = "x";

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Result<Self, BoxedError> {
        crate::asset::load_text(&bytes).map(X)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XS(pub i32);

impl FileAsset for XS {
    const EXTENSION: &'static str = "x";

    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Result<Self, BoxedError> {
        crate::asset::load_text(&bytes).map(XS)
    }

    const HOT_RELOADED: bool = false;
}

#[derive(Debug)]
pub struct Y(pub i32);

impl Asset for Y {
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Y, BoxedError> {
        Ok(Y(cache.load::<X>(id)?.read().0))
    }
}

pub struct Z(pub i32);

impl Asset for Z {
    fn load(cache: &AssetCache, id: &SharedString) -> Result<Z, BoxedError> {
        Ok(Z(cache.load::<Y>(id)?.read().0))
    }
}

mod asset_cache {
    use super::{X, Y};
    use crate::AssetCache;

    #[test]
    fn new_with_valid_path() {
        let cache = AssetCache::new("assets");
        assert!(cache.is_ok());
    }

    #[test]
    fn new_with_invalid_path() {
        let cache = AssetCache::new("asset");
        assert!(cache.is_err());
    }

    #[test]
    fn new_with_valid_file() {
        let cache = AssetCache::new("src/lib.rs");
        assert!(cache.is_err());
    }

    #[test]
    fn load() {
        let cache = AssetCache::new("assets").unwrap();

        assert_eq!(*cache.load::<X>("test.cache").unwrap().read(), X(42));
        assert!(cache.contains::<X>("test.cache"));
    }

    #[test]
    fn load_owned() {
        let cache = AssetCache::new("assets").unwrap();

        assert_eq!(cache.load_owned::<X>("test.cache").unwrap(), X(42));
        assert!(!cache.contains::<X>("test.cache"));
    }

    #[test]
    fn get_cached() {
        let cache = AssetCache::new("assets").unwrap();

        assert!(cache.get_cached::<X>("test.cache").is_none());
        cache.load::<X>("test.cache").unwrap();
        assert_eq!(*cache.get_cached::<X>("test.cache").unwrap().read(), X(42));
    }

    #[test]
    fn get_or_insert() {
        let cache = AssetCache::new("assets").unwrap();

        assert!(cache.get_cached::<i32>("test.xxx").is_none());
        let handle = cache.get_or_insert::<i32>("test.xxx", 5);
        assert_eq!(*handle.read(), 5);
    }

    #[test]
    fn errors() {
        let cache = AssetCache::new("assets").unwrap();

        let err = cache.load::<String>("test.missing").unwrap_err();
        assert!(err.reason().downcast_ref::<std::io::Error>().is_some());

        let err = cache.load::<Y>("test.a").unwrap_err();
        assert_eq!(err.id(), "test.a");
        let err = err.reason().downcast_ref::<crate::Error>().unwrap();
        assert_eq!(err.id(), "test.a");
        assert!(
            err.reason()
                .downcast_ref::<std::num::ParseIntError>()
                .is_some()
        );

        let err = cache.load_owned::<X>("test.a").unwrap_err();
        assert_eq!(err.id(), "test.a");
        assert!(
            err.reason()
                .downcast_ref::<std::num::ParseIntError>()
                .is_some()
        );
    }

    #[test]
    fn load_dir_ok() {
        let cache = AssetCache::new("assets").unwrap();

        assert!(!cache.contains::<crate::Directory<X>>("test"));
        let mut loaded: Vec<_> = cache
            .load_dir::<X>("test")
            .unwrap()
            .read()
            .iter(&cache)
            .filter_map(|x| Some(x.ok()?.read().0))
            .collect();
        assert!(cache.contains::<crate::Directory<X>>("test"));

        loaded.sort();
        assert_eq!(loaded, [-7, 42]);
    }

    #[test]
    fn load_dir_all() {
        let cache = AssetCache::new("assets").unwrap();

        let dir = cache.load_dir::<X>("test").unwrap().read();
        let mut loaded: Vec<_> = dir.ids().map(|id| (id, cache.load::<X>(id))).collect();
        loaded.sort_by_key(|i| i.0);
        let mut loaded = loaded.into_iter();

        let (id, x) = loaded.next().unwrap();
        assert_eq!(id, "test.a");
        assert!(x.is_err());

        let (id, x) = loaded.next().unwrap();
        assert_eq!(id, "test.b");
        assert_eq!(*x.unwrap().read(), X(-7));

        let (id, x) = loaded.next().unwrap();
        assert_eq!(id, "test.cache");
        assert_eq!(*x.unwrap().read(), X(42));

        assert!(loaded.next().is_none());
    }

    #[test]
    fn fallback() {
        let fallback = AssetCache::new("assets").unwrap();
        let cache = AssetCache::with_fallback(fallback.clone());

        let handle = fallback.load::<X>("test.cache").unwrap();

        assert_eq!(*handle.read(), X(42));
        assert_eq!(*cache.get_cached::<X>("test.cache").unwrap().read(), X(42));
        assert!(cache.contains::<X>("test.cache"));

        assert_eq!(*cache.load::<X>("test.b").unwrap().read(), X(-7));
        assert!(fallback.get_cached::<X>("test.b").is_none());

        drop(cache);

        assert_eq!(*handle.read(), X(42));
    }
}

mod handle {
    use super::*;

    #[test]
    fn id() {
        let cache = AssetCache::new("assets").unwrap();
        let handle = cache.load::<X>("test.cache").unwrap();
        assert_eq!(handle.id(), "test.cache");
    }

    #[test]
    fn same_handle() {
        let cache = AssetCache::new("assets").unwrap();
        let handle1 = cache.load::<X>("test.cache").unwrap();
        let handle2 = cache.load::<X>("test.cache").unwrap();
        assert!(std::ptr::eq(handle1, handle2));
    }

    #[test]
    fn untyped() {
        let cache = AssetCache::new("assets").unwrap();
        let handle = cache.load_expect::<X>("test.cache").as_untyped();

        assert!(handle.is::<X>());
        assert_eq!(handle.id(), "test.cache");
        assert_eq!(*handle.downcast_ref::<X>().unwrap().read(), X(42));
        assert_eq!(*handle.read().downcast_ref::<X>().unwrap(), X(42));
    }
}

#[test]
fn weird_id() {
    let cache = AssetCache::new("assets").unwrap();

    let err = cache.load::<X>("test/cache").unwrap_err();
    assert_eq!(err.reason().to_string(), "invalid id");
}
