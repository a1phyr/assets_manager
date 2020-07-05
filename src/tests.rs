use crate::{Asset, loader};

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


mod asset_cache {
    use crate::AssetCache;
    use super::X;

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
    }

    #[test]
    fn load_owned() {
        let cache = AssetCache::new("assets").unwrap();

        assert_eq!(cache.load_owned::<X>("test.cache").unwrap(), X(42));
    }

    #[test]
    fn load_cached() {
        let cache = AssetCache::new("assets").unwrap();

        assert!(cache.load_cached::<X>("test.cache").is_none());
        cache.load::<X>("test.cache").unwrap();
        assert_eq!(*cache.load_cached::<X>("test.cache").unwrap().read(), X(42));
    }

    #[test]
    fn reload_set_flag() {
        let cache = AssetCache::new("assets").unwrap();

        let mut asset = cache.load_expect::<X>("test.cache");
        assert!(!asset.reloaded());
        cache.force_reload::<X>("test.cache").unwrap();
        assert!(asset.reloaded());
        assert!(!asset.reloaded());
    }

    #[test]
    fn load_dir_ok() {
        let cache = AssetCache::new("assets").unwrap();

        let mut loaded: Vec<_> = cache.load_dir::<X>("test").unwrap()
            .iter().map(|x| x.read().0).collect();
        loaded.sort();
        assert_eq!(loaded, [-7, 42]);
    }

    #[test]
    fn load_dir_all() {
        let cache = AssetCache::new("assets").unwrap();

        let mut loaded: Vec<_> = cache.load_dir::<X>("test").unwrap().iter_all().collect();
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
        assert!(cache.load_cached::<X>("test.cache").is_some());
        assert_eq!(cache.take("test.cache"), Some(X(42)));
        assert!(cache.load_cached::<X>("test.cache").is_none());
    }

    #[test]
    fn remove() {
        let mut cache = AssetCache::new("assets").unwrap();

        cache.load::<X>("test.cache").unwrap();
        assert!(cache.load_cached::<X>("test.cache").is_some());
        cache.remove::<X>("test.cache");
        assert!(cache.load_cached::<X>("test.cache").is_none());
    }
}

mod cache_entry {
    use std::sync::{Arc, Mutex};
    use crate::lock::CacheEntry;

    #[derive(Clone)]
    struct DropCounter(Arc<Mutex<usize>>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            let mut count = self.0.lock().unwrap();
            *count += 1;
        }
    }

    #[test]
    fn drop_inner() {
        let count = DropCounter(Arc::new(Mutex::new(0)));

        let entry_1 = CacheEntry::new(count.clone());
        let entry_2 = CacheEntry::new(count.clone());
        assert_eq!(*count.0.lock().unwrap(), 0);
        drop(entry_1);
        assert_eq!(*count.0.lock().unwrap(), 1);
        drop(entry_2);
        assert_eq!(*count.0.lock().unwrap(), 2);
    }

    #[test]
    fn read() {
        let val = rand::random::<i32>();

        let entry = CacheEntry::new(val);
        let guard = unsafe { entry.get_ref::<i32>() };

        assert_eq!(*guard.read(), val);
    }

    #[test]
    fn write() {
        let x = rand::random::<i32>();
        let y = rand::random::<i32>();

        let entry = CacheEntry::new(x);
        unsafe {
            let guard = entry.write(y);
            assert_eq!(*guard.read(), y);
            let guard = entry.get_ref::<i32>();
            assert_eq!(*guard.read(), y);
        }
    }

    #[test]
    fn into_inner() {
        let x = rand::random::<i32>();

        let entry = CacheEntry::new(x);
        let y = unsafe { entry.into_inner::<i32>() };

        assert_eq!(x, y);
    }

    #[test]
    fn ptr_eq() {
        let x = rand::random::<i32>();

        let entry = CacheEntry::new(x);
        unsafe {
            let ref_1 = entry.get_ref::<i32>();
            let ref_2 = entry.get_ref::<i32>();
            assert!(ref_1.ptr_eq(&ref_2));
        }
    }
}
