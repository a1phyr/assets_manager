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
    fn parse_loader() {
        let n = rand::random::<i32>();
        let raw = raw(&format!("{}", n));

        let loaded: i32 = ParseLoader::load(raw).unwrap();

        assert_eq!(loaded, n);
    }

    #[cfg(feature = "serde")]
    use serde::{Serialize, Deserialize};
    #[cfg(feature = "serde")]
    use rand::{
        Rng,
        distributions::{Distribution, Standard},
    };

    #[cfg(feature = "serde")]
    #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[cfg(feature = "serde")]
    impl Distribution<Point> for Standard {
        fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Point {
            Point {
                x: rng.gen(),
                y: rng.gen(),
            }
        }
    }

    #[cfg(feature = "serde")]
    macro_rules! test_loader {
        ($name:ident, $loader:ty, $ser:expr) => {
            #[test]
            fn $name() {
                let point = rand::random::<Point>();
                let raw = ($ser)(&point);

                let loaded: Point = <$loader>::load(raw).unwrap();

                assert_eq!(loaded, point);
            }
        }
    }

    #[cfg(feature = "bincode")]
    test_loader!(bincode_loader, BincodeLoader, |p| serde_bincode::serialize(p).unwrap());

    #[cfg(feature = "cbor")]
    test_loader!(cbor_loader, CborLoader, |p| serde_cbor::to_vec(p).unwrap());

    #[cfg(feature = "json")]
    test_loader!(json_loader, JsonLoader, |p| serde_json::to_vec(p).unwrap());

    #[cfg(feature = "ron")]
    test_loader!(ron_loader, RonLoader, |p| serde_ron::ser::to_string(p).unwrap().into_bytes());

    #[cfg(feature = "toml")]
    test_loader!(toml_loader, TomlLoader, |p| serde_toml::ser::to_vec(p).unwrap());

    #[cfg(feature = "yaml")]
    test_loader!(yaml_loader, YamlLoader, |p| serde_yaml::to_vec(p).unwrap());
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
