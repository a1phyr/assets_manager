use std::borrow::Cow;

use crate::{BoxedError, FileAsset};
use ab_glyph::{FontArc, FontVec};

#[cfg_attr(docsrs, doc(cfg(feature = "ab_glyph")))]
impl FileAsset for FontVec {
    const EXTENSIONS: &'static [&'static str] = &["ttf", "otf"];

    fn from_bytes(bytes: Cow<[u8]>) -> Result<Self, BoxedError> {
        Ok(FontVec::try_from_vec(bytes.into_owned())?)
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "ab_glyph")))]
impl FileAsset for FontArc {
    const EXTENSIONS: &'static [&'static str] = &["ttf", "otf"];

    fn from_bytes(bytes: Cow<[u8]>) -> Result<Self, BoxedError> {
        Ok(FontArc::try_from_vec(bytes.into_owned())?)
    }
}
