use crate::{Asset, loader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct X(i32);

impl From<i32> for X {
    fn from(n: i32) -> X {
        X(n)
    }
}

impl Asset for X {
    type Loader = loader::FromOther<i32, loader::ParseLoader>;
    const EXT: &'static str = "";
}


mod loaders {
    use crate::loader::*;

    fn raw(s: &str) -> Vec<u8> {
        s.to_string().into_bytes()
    }

    #[test]
    fn string_loader() {
        let raw = raw("Hello World!");
        let loaded = StringLoader::load(raw).unwrap();

        assert_eq!(loaded, "Hello World!");
    }

    #[test]
    fn load_or_default() {
        let raw = raw("a");

        let loaded: i32 = LoadOrDefault::<ParseLoader>::load(raw).unwrap();

        assert_eq!(loaded, 0);
    }

    #[test]
    fn parse_loader() {
        let n = rand::random::<i32>();
        let raw = raw(&format!("{}", n));

        let loaded: i32 = ParseLoader::load(raw).unwrap();

        assert_eq!(loaded, n);
    }

    #[test]
    fn from_other() {
        use super::X;

        let n = rand::random::<i32>();
        let raw = raw(&format!("{}", n));

        let loaded: X = FromOther::<i32, ParseLoader>::load(raw).unwrap();

        assert_eq!(loaded, X(n));
    }

    cfg_if::cfg_if! { if #[cfg(feature = "serde")] {
        use serde::{Serialize, Deserialize};
        use rand::{
            Rng,
            distributions::{Distribution, Standard},
        };

        #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
        struct Point {
            x: i32,
            y: i32,
        }

        impl Distribution<Point> for Standard {
            fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Point {
                Point {
                    x: rng.gen(),
                    y: rng.gen(),
                }
            }
        }

        macro_rules! test_loader {
            ($name:ident, $loader:ty, $ser:expr) => {
                #[test]
                fn $name() {
                    let point = rand::random::<Point>();
                    let raw = ($ser)(&point).unwrap();

                    let loaded: Point = <$loader>::load(raw).unwrap();

                    assert_eq!(loaded, point);
                }
            }
        }
    }}

    #[cfg(feature = "bincode")]
    test_loader!(bincode_loader, BincodeLoader, serde_bincode::serialize);

    #[cfg(feature = "cbor")]
    test_loader!(cbor_loader, CborLoader, serde_cbor::to_vec);

    #[cfg(feature = "json")]
    test_loader!(json_loader, JsonLoader, serde_json::to_vec);

    #[cfg(feature = "msgpack")]
    test_loader!(msgpack_loader, MessagePackLoader, serde_msgpack::encode::to_vec);

    #[cfg(feature = "ron")]
    test_loader!(ron_loader, RonLoader, |p| serde_ron::ser::to_string(p).map(String::into_bytes));

    #[cfg(feature = "toml")]
    test_loader!(toml_loader, TomlLoader, serde_toml::ser::to_vec);

    #[cfg(feature = "yaml")]
    test_loader!(yaml_loader, YamlLoader, serde_yaml::to_vec);
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
    fn load_cached() {
        let x = X(rand::random());

        let cache = AssetCache::new(".").unwrap();

        assert!(cache.load_cached::<X>("").is_none());
        cache.add_asset(String::new(), x);
        assert_eq!(*cache.load_cached::<X>("").unwrap().read(), x);
    }

    #[test]
    fn take() {
        let x = X(rand::random());

        let mut cache = AssetCache::new(".").unwrap();

        cache.add_asset(String::new(), x);
        assert!(cache.load_cached::<X>("").is_some());
        assert_eq!(cache.take(""), Some(x));
        assert!(cache.load_cached::<X>("").is_none());
    }

    #[test]
    fn remove() {
        let x = X(rand::random());

        let mut cache = AssetCache::new(".").unwrap();

        cache.add_asset(String::new(), x);
        assert!(cache.load_cached::<X>("").is_some());
        cache.remove::<X>("");
        assert!(cache.load_cached::<X>("").is_none());
    }
}

mod cache_entry {
    use std::sync::Mutex;
    use crate::lock::CacheEntry;

    struct DropCounter<'a> {
        count: &'a Mutex<usize>,
    }

    impl Drop for DropCounter<'_> {
        fn drop(&mut self) {
            let mut count = self.count.lock().unwrap();
            *count += 1;
        }
    }

    #[test]
    fn drop_inner() {
        let count = &Mutex::new(0);

        let entry_1 = CacheEntry::new(DropCounter { count });
        let entry_2 = CacheEntry::new(DropCounter { count });
        assert_eq!(*count.lock().unwrap(), 0);
        drop(entry_1);
        assert_eq!(*count.lock().unwrap(), 1);
        drop(entry_2);
        assert_eq!(*count.lock().unwrap(), 2);
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
