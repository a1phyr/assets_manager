//! `rodio` integration for `assets_manager`
//!
//! This crate provides wrappers around `rodio` sounds types that implement
//! `assets_manager` traits.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs, missing_debug_implementations)]
#![forbid(unsafe_code)]

use assets_manager::{BoxedError, FileAsset, SharedBytes};
use rodio::decoder::{Decoder, DecoderError};
use std::{borrow::Cow, io};

#[cfg(test)]
mod tests;

const AVAILABLE_EXTENSIONS: &[&str] = &[
    #[cfg(any(feature = "vorbis", feature = "symphonia-vorbis"))]
    "ogg",
    #[cfg(any(feature = "minimp3", feature = "symphonia-mp3"))]
    "mp3",
    #[cfg(any(feature = "flac", feature = "symphonia-flac"))]
    "flac",
    #[cfg(any(feature = "wav", feature = "symphonia-wav"))]
    "wav",
];

macro_rules! sound_assets {
    (
        $(
            #[doc = $doc:literal]
            $( #[cfg( $( $cfg:tt )* )] )?
            struct $name:ident => (
                $decoder:path,
                $ext:expr,
            );
        )*
    ) => {
        $(
            #[doc = $doc]
            $( #[cfg($($cfg)*)] )?
            $( #[cfg_attr(docsrs, doc(cfg($($cfg)*)))] )?
            #[derive(Clone, Debug)]
            pub struct $name(SharedBytes);

            $( #[cfg($($cfg)*)] )?
            $( #[cfg_attr(docsrs, doc(cfg($($cfg)*)))] )?
            impl FileAsset for $name {
                const EXTENSIONS: &'static [&'static str] = $ext;

                fn from_bytes(bytes: Cow<[u8]>) -> Result<Self, BoxedError> {
                    Ok($name::new(bytes.into())?)
                }
            }

            $( #[cfg($($cfg)*)] )?
            impl $name {
                /// Creates a new sound from raw bytes.
                #[inline]
                pub fn new(bytes: SharedBytes) -> Result<$name, DecoderError> {
                    // We have to clone the bytes here because `Decoder::new`
                    // requires a 'static lifetime, but it should be cheap
                    // anyway.
                    let _ = $decoder(io::Cursor::new(bytes.clone()))?;
                    Ok($name(bytes))
                }

                /// Creates a [`Decoder`] that can be send to `rodio` to play
                /// sounds.
                #[inline]
                pub fn decoder(self) -> Decoder<io::Cursor<SharedBytes>> {
                    $decoder(io::Cursor::new(self.0)).unwrap()
                }

                #[inline]
                /// Returns a bytes slice of the sound content.
                pub fn as_bytes(&self) -> &[u8] {
                    &self.0
                }

                /// Convert the sound back to raw bytes.
                #[inline]
                pub fn into_bytes(self) -> SharedBytes {
                    self.0
                }
            }

            $( #[cfg($($cfg)*)] )?
            impl AsRef<[u8]> for $name {
                fn as_ref(&self) -> &[u8] {
                    &self.0
                }
            }
        )*
    }
}

sound_assets! {
    /// Loads FLAC sounds
    #[cfg(any(feature = "flac", feature = "symphonia-flac"))]
    struct Flac => (
        Decoder::new_flac,
        &["flac"],
    );

    /// Loads MP3 sounds
    #[cfg(any(feature = "minimp3", feature = "symphonia-mp3"))]
    struct Mp3 => (
        Decoder::new_mp3,
        &["mp3"],
    );

    /// Loads Vorbis sounds
    #[cfg(any(feature = "vorbis", feature = "symphonia-vorbis"))]
    struct Vorbis => (
        Decoder::new_vorbis,
        &["ogg"],
    );

    /// Loads WAV sounds
    #[cfg(any(feature = "wav", feature = "symphonia-wav"))]
    struct Wav => (
        Decoder::new_wav,
        &["wav"],
    );

    /// Loads sounds of any enabled kind
    struct Sound => (
        Decoder::new,
        AVAILABLE_EXTENSIONS,
    );
}
