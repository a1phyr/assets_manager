use std::borrow::Cow;

use crate::{loader, Asset, BoxedError};
use ab_glyph::{FontArc, FontVec};

#[cfg_attr(docsrs, doc(cfg(feature = "ab_glyph")))]
impl loader::Loader<FontVec> for loader::FontLoader {
    fn load(content: Cow<[u8]>, _: &str) -> Result<FontVec, BoxedError> {
        Ok(FontVec::try_from_vec(content.into_owned())?)
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "ab_glyph")))]
impl loader::Loader<FontArc> for loader::FontLoader {
    fn load(content: Cow<[u8]>, _: &str) -> Result<FontArc, BoxedError> {
        Ok(FontArc::try_from_vec(content.into_owned())?)
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "ab_glyph")))]
impl Asset for FontVec {
    type Loader = loader::FontLoader;
    const EXTENSIONS: &'static [&'static str] = &["ttf", "otf"];
}

#[cfg_attr(docsrs, doc(cfg(feature = "ab_glyph")))]
impl Asset for FontArc {
    type Loader = loader::FontLoader;
    const EXTENSIONS: &'static [&'static str] = &["ttf", "otf"];
}
