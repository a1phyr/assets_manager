use crate::*;
use std::sync::Arc;

#[test]
fn string_assets_ok() {
    let cache = AssetCache::new("assets").unwrap();

    let contents = "Hello World!\n";

    std::fs::write("assets/test/string_base.txt", "Hello World!\n").unwrap();

    assert_eq!(
        &**cache.load_expect::<String>("test.string_base").read(),
        contents
    );
    assert_eq!(
        &**cache.load_expect::<Box<str>>("test.string_base").read(),
        contents
    );
    assert_eq!(
        &**cache.load_expect::<SharedString>("test.string_base").read(),
        contents
    );
    assert_eq!(
        &**cache.load_expect::<Arc<str>>("test.string_base").read(),
        contents
    );
}

#[test]
fn string_utf8_err() {
    let cache = AssetCache::new("assets").unwrap();

    std::fs::write("assets/test/invalid.txt", b"e\xa2").unwrap();

    let err = cache.load::<String>("test.invalid").unwrap_err();
    err.downcast::<std::str::Utf8Error>().unwrap();
}

#[cfg(feature = "gltf")]
mod gltf {
    use crate::*;

    #[test]
    pub fn gltf() {
        let cache = AssetCache::new("assets").unwrap();
        cache.load::<asset::Gltf>("test.gltf.box").unwrap();
    }

    #[test]
    pub fn gltf_bin() {
        let cache = AssetCache::new("assets").unwrap();
        cache.load::<asset::Gltf>("test.gltf.box-bin").unwrap();
    }

    #[test]
    pub fn gltf_embedded() {
        let cache = AssetCache::new("assets").unwrap();
        cache.load::<asset::Gltf>("test.gltf.box-embedded").unwrap();
    }

    #[test]
    pub fn gltf_dir() {
        let cache = AssetCache::new("assets").unwrap();
        let dir = cache.load_dir::<asset::Gltf>("test.gltf").unwrap();

        let dir = dir.read();
        let mut ids: Vec<_> = dir.ids().collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            [
                "test.gltf.box",
                "test.gltf.box-bin",
                "test.gltf.box-embedded"
            ]
        )
    }
}
