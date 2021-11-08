#[allow(unused_imports)]
use crate::*;

macro_rules! sound_test {
    (
        $(
            #[cfg(feature = $feat:literal)]
            $name:ident => $kind:path,
        )*
    ) => {
        $(
            #[test]
            #[cfg(feature = $feat)]
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
    test_mp3 => asset::Mp3,

    #[cfg(feature ="vorbis")]
    test_vorbis => asset::Vorbis,

    #[cfg(feature ="wav")]
    test_wav => asset::Wav,
}
