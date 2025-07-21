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
                let cache = assets_manager::AssetCache::new("../../assets").expect("oops");
                assert!(cache.load::<$kind>("test.sounds.silence").is_ok());
            }
        )*
    };
}

sound_test! {
    #[cfg(any(feature = "flac", feature = "claxon"))]
    test_flac => crate::Flac,

    #[cfg(any(feature = "mp3", feature = "minimp3"))]
    test_mp3 => crate::Mp3,

    #[cfg(any(feature = "vorbis", feature = "lewton"))]
    test_vorbis => crate::Vorbis,

    #[cfg(any(feature = "wav", feature = "hound"))]
    test_wav => crate::Wav,
}
