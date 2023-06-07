//! `kira` integration for `assets_manager`
//!
//! This crate provides wrappers around `kira` sounds types that implement
//! `assets_manager` traits.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

const AVAILABLE_EXTENSIONS: &[&str] = &[
    #[cfg(feature = "ogg")]
    "ogg",
    #[cfg(feature = "mp3")]
    "mp3",
    #[cfg(feature = "flac")]
    "flac",
    #[cfg(feature = "wav")]
    "wav",
];

pub use static_sound::StaticSound;
pub use streaming::StreamingSound;

mod static_sound {
    use assets_manager::{loader, Asset};
    use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
    use std::io::Cursor;

    /// A wrapper around [`StaticSoundData`] that implements [`Asset`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// use kira::manager::{backend::DefaultBackend, AudioManager, AudioManagerSettings};
    /// use assets_manager_kira::StaticSound;
    ///
    /// let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())?;
    /// let cache = assets_manager::AssetCache::new("assets")?;
    ///
    /// loop {
    ///     let sound_data = cache.load::<StaticSound>("example.audio.beep")?;
    ///     manager.play(sound_data.cloned())?;
    ///     std::thread::sleep(std::time::Duration::from_secs(1));
    /// }
    ///
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    #[derive(Clone)]
    #[repr(transparent)]
    pub struct StaticSound(pub StaticSoundData);

    impl StaticSound {
        /// Returns the duration of the audio.
        pub fn duration(&self) -> std::time::Duration {
            self.0.duration()
        }

        /// Returns a clone of the `StaticSound` with the specified settings.
        pub fn with_settings(&self, settings: StaticSoundSettings) -> Self {
            Self(self.0.with_settings(settings))
        }
    }

    impl loader::Loader<StaticSound> for loader::SoundLoader {
        fn load(
            content: std::borrow::Cow<[u8]>,
            _: &str,
        ) -> Result<StaticSound, assets_manager::BoxedError> {
            let sound = StaticSoundData::from_cursor(
                Cursor::new(content.into_owned()),
                StaticSoundSettings::default(),
            )?;
            Ok(StaticSound(sound))
        }
    }

    impl Asset for StaticSound {
        const EXTENSIONS: &'static [&'static str] = crate::AVAILABLE_EXTENSIONS;
        type Loader = loader::SoundLoader;
    }

    impl kira::sound::SoundData for StaticSound {
        type Error = <StaticSoundData as kira::sound::SoundData>::Error;
        type Handle = <StaticSoundData as kira::sound::SoundData>::Handle;

        #[inline]
        fn into_sound(self) -> Result<(Box<dyn kira::sound::Sound>, Self::Handle), Self::Error> {
            self.0.into_sound()
        }
    }

    impl From<StaticSound> for StaticSoundData {
        #[inline]
        fn from(sound: StaticSound) -> Self {
            sound.0
        }
    }

    impl std::fmt::Debug for StaticSound {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.fmt(f)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(docsrs, doc(cfg(not(target_arch = "wasm32"))))]
mod streaming {
    use assets_manager::{loader, Asset};
    use kira::sound::{
        streaming::{StreamingSoundData, StreamingSoundSettings},
        FromFileError,
    };
    use std::io::Cursor;

    /// A wrapper around [`StreamingSoundData`] that implements [`Asset`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// use kira::manager::{backend::DefaultBackend, AudioManager, AudioManagerSettings};
    /// use assets_manager_kira::StreamingSound;
    ///
    /// let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())?;
    /// let cache = assets_manager::AssetCache::new("assets")?;
    ///
    /// loop {
    ///     let sound_data = cache.load::<StreamingSound>("example.audio.beep")?;
    ///     manager.play(sound_data.cloned())?;
    ///     std::thread::sleep(std::time::Duration::from_secs(1));
    /// }
    ///
    /// # Ok::<_, Box<dyn std::error::Error>>(())
    /// ```
    #[derive(Clone)]
    pub struct StreamingSound {
        /// Settings for the sound.
        pub settings: StreamingSoundSettings,
        bytes: assets_manager::SharedBytes,
    }

    impl StreamingSound {
        /// Returns a clone of the `StreamingSound` with the specified settings.
        pub fn with_settings(&self, settings: StreamingSoundSettings) -> Self {
            Self {
                settings,
                bytes: self.bytes.clone(),
            }
        }

        fn try_into_kira(self) -> Result<StreamingSoundData<FromFileError>, FromFileError> {
            StreamingSoundData::from_cursor(Cursor::new(self.bytes), self.settings)
        }
    }

    impl loader::Loader<StreamingSound> for loader::SoundLoader {
        fn load(
            content: std::borrow::Cow<[u8]>,
            _: &str,
        ) -> Result<StreamingSound, assets_manager::BoxedError> {
            let bytes = assets_manager::SharedBytes::from(content);
            let settings = StreamingSoundSettings::default();

            // Check that the audio file is valid.
            let _ = StreamingSoundData::from_cursor(Cursor::new(bytes.clone()), settings)?;

            Ok(StreamingSound { settings, bytes })
        }
    }

    impl Asset for StreamingSound {
        const EXTENSIONS: &'static [&'static str] = crate::AVAILABLE_EXTENSIONS;
        type Loader = loader::SoundLoader;
    }

    impl kira::sound::SoundData for StreamingSound {
        type Error = <StreamingSoundData<FromFileError> as kira::sound::SoundData>::Error;
        type Handle = <StreamingSoundData<FromFileError> as kira::sound::SoundData>::Handle;

        #[inline]
        fn into_sound(self) -> Result<(Box<dyn kira::sound::Sound>, Self::Handle), Self::Error> {
            self.try_into_kira()?.into_sound()
        }
    }

    impl From<StreamingSound> for StreamingSoundData<FromFileError> {
        fn from(sound: StreamingSound) -> Self {
            sound.try_into_kira().expect("reading succeded earlier")
        }
    }

    impl std::fmt::Debug for StreamingSound {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("StreamingSound")
                .field("settings", &self.settings)
                .finish_non_exhaustive()
        }
    }
}
