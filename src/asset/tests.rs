#[allow(unused_imports)]
use crate::*;

macro_rules! sound_test {
    (
        $(
            $( #[$attr:meta] )*
            $name:ident => $kind:path,
        )*
    ) => {
        $(
            #[test]
            $( #[$attr] )*
            fn $name() {
                let cache = AssetCache::new("assets").unwrap();
                assert!(cache.load::<$kind>("test.sounds.silence").is_ok());
            }
        )*
    };
}

sound_test! {
    #[cfg(feature ="flac")]
    test_flac => asset::Flac,

    #[cfg(feature ="mp3")]
    #[ignore = "Blocks on https://github.com/RustAudio/rodio/issues/411"]
    test_mp3 => asset::Mp3,

    #[cfg(feature ="vorbis")]
    test_vorbis => asset::Vorbis,

    #[cfg(feature ="wav")]
    test_wav => asset::Wav,
}

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
    let dir = cache.load_dir::<asset::Gltf>("test.gltf", false).unwrap();

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
