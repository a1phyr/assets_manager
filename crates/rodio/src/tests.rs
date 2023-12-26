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
    #[cfg(any(feature = "flac", feature = "symphonia-flac"))]
    test_flac => crate::Flac,

    // Disabled for feature "minimp3" because of soundness issues
    #[cfg(feature = "symphonia-mp3")]
    test_mp3 => crate::Mp3,

    #[cfg(any(feature = "vorbis", feature = "symphonia-vorbis"))]
    test_vorbis => crate::Vorbis,

    #[cfg(any(feature = "wav", feature = "symphonia-wav"))]
    test_wav => crate::Wav,
}
