use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X(pub i32);

impl From<i32> for X {
    fn from(n: i32) -> X {
        X(n)
    }
}

impl Asset for X {
    type Loader = loader::LoadFrom<i32, loader::ParseLoader>;
    const EXTENSION: &'static str = "x";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XS(pub i32);

impl From<i32> for XS {
    fn from(n: i32) -> XS {
        XS(n)
    }
}

impl Asset for XS {
    type Loader = loader::LoadFrom<i32, loader::ParseLoader>;
    const EXTENSION: &'static str = "x";
    const HOT_RELOADED: bool = false;
}

impl asset::NotHotReloaded for XS {}

#[derive(Debug)]
pub struct Y(pub i32);

impl Compound for Y {
    fn load(cache: AnyCache, id: &SharedString) -> Result<Y, BoxedError> {
        Ok(Y(cache.load::<X>(id)?.read().0))
    }
}

pub struct Z(pub i32);

impl Compound for Z {
    fn load(cache: AnyCache, id: &SharedString) -> Result<Z, BoxedError> {
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
        assert_eq!(*handle.get(), 5);
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
        assert!(err
            .reason()
            .downcast_ref::<std::num::ParseIntError>()
            .is_some());

        let err = cache.load_owned::<X>("test.a").unwrap_err();
        assert_eq!(err.id(), "test.a");
        assert!(err
            .reason()
            .downcast_ref::<std::num::ParseIntError>()
            .is_some());
    }

    #[test]
    fn load_dir_ok() {
        let cache = AssetCache::new("assets").unwrap();

        assert!(!cache.contains_dir::<X>("test", false));
        let mut loaded: Vec<_> = cache
            .load_dir::<X>("test", false)
            .unwrap()
            .iter(cache.as_any_cache())
            .filter_map(|x| Some(x.ok()?.read().0))
            .collect();
        assert!(cache.contains_dir::<X>("test", false));

        loaded.sort();
        assert_eq!(loaded, [-7, 42]);
    }

    #[test]
    fn load_dir_all() {
        let cache = AssetCache::new("assets").unwrap();

        let mut loaded: Vec<_> = cache
            .load_dir::<X>("test", false)
            .unwrap()
            .ids()
            .map(|id| (id, cache.load::<X>(id)))
            .collect();
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
    fn take() {
        let mut cache = AssetCache::new("assets").unwrap();

        cache.load::<X>("test.cache").unwrap();
        assert!(cache.contains::<X>("test.cache"));
        assert_eq!(cache.take("test.cache"), Some(X(42)));
        assert!(!cache.contains::<X>("test.cache"));
    }

    #[test]
    fn remove() {
        let mut cache = AssetCache::new("assets").unwrap();

        cache.load::<X>("test.cache").unwrap();
        assert!(cache.contains::<X>("test.cache"));
        cache.remove::<X>("test.cache");
        assert!(!cache.contains::<X>("test.cache"));
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
        assert!(handle1.same_handle(&handle2));
    }

    #[test]
    fn get() {
        let cache = AssetCache::new("assets").unwrap();
        let handle = cache.load::<XS>("test.cache").unwrap();
        assert_eq!(*handle.get(), XS(42));
    }
}
