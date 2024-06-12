#[allow(unused_imports)]
use crate::*;

#[cfg(feature = "gltf")]
#[test]
pub fn gltf() {
    let cache = AssetCache::new("assets").unwrap();
    cache.load::<asset::Gltf>("test.gltf.box").unwrap();
}

#[cfg(feature = "gltf")]
#[test]
pub fn gltf_bin() {
    let cache = AssetCache::new("assets").unwrap();
    cache.load::<asset::Gltf>("test.gltf.box-bin").unwrap();
}

#[cfg(feature = "gltf")]
#[test]
pub fn gltf_embedded() {
    let cache = AssetCache::new("assets").unwrap();
    cache.load::<asset::Gltf>("test.gltf.box-embedded").unwrap();
}

#[cfg(feature = "gltf")]
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
