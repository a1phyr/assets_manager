use crate::{loader, utils, AnyCache, Asset, BoxedError, Compound, SharedString};
use std::path;

#[cfg_attr(docsrs, doc(cfg(feature = "gltf")))]
impl Asset for gltf::Gltf {
    const EXTENSIONS: &'static [&'static str] = &["glb", "gltf"];
    type Loader = loader::GltfLoader;
}

/// Loads glTF 3D assets.
///
/// This struct provides access to the raw glTF document, and methods to
/// access buffers, views and images.
#[derive(Debug)]
#[cfg_attr(docsrs, doc(cfg(feature = "gltf")))]
pub struct Gltf {
    /// The glTF document.
    pub document: gltf::Document,

    images: Vec<image::DynamicImage>,
    buffers: Vec<Vec<u8>>,
}

impl Gltf {
    /// Retreives the content of a buffer.
    pub fn get_buffer(&self, buffer: &gltf::Buffer) -> &[u8] {
        self.get_buffer_by_index(buffer.index())
    }

    /// Retreives the content of a buffer by its index.
    pub fn get_buffer_by_index(&self, index: usize) -> &[u8] {
        &self.buffers[index]
    }

    /// Retreives the content of a buffer view.
    pub fn get_buffer_view(&self, view: &gltf::buffer::View) -> &[u8] {
        let buffer = self.get_buffer(&view.buffer());
        let start = view.offset();
        let end = start + view.length();
        &buffer[start..end]
    }

    /// Retreives the content of an image.
    pub fn get_image(&self, image: &gltf::Image) -> &image::DynamicImage {
        self.get_image_by_index(image.index())
    }

    /// Retreives the content of an image by its index.
    pub fn get_image_by_index(&self, index: usize) -> &image::DynamicImage {
        &self.images[index]
    }
}

#[derive(Clone)]
struct Bin(Vec<u8>);

impl Asset for Bin {
    const EXTENSION: &'static str = "bin";
    type Loader = loader::LoadFrom<Vec<u8>, loader::BytesLoader>;
}

impl From<Vec<u8>> for Bin {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

enum UriContent<'a> {
    Bin {
        mime_type: Option<&'a str>,
        content: Vec<u8>,
    },
    File {
        id: String,
        ext: &'a str,
    },
}

impl<'a> UriContent<'a> {
    fn parse_uri(
        base_id: &str,
        uri: &'a str,
        mime_type: Option<&'a str>,
    ) -> Result<Self, BoxedError> {
        if let Some(uri) = uri.strip_prefix("data:") {
            let mut data = uri.split(";base64,");

            let fst = match data.next() {
                Some(fst) => fst,
                None => return Err("Unsupported".into()),
            };

            let (mime_type, b64) = match data.next() {
                Some(data) => (mime_type.or(Some(fst)), data),
                None => (mime_type, fst),
            };

            #[allow(deprecated)]
            let content = base64::decode(b64)?;
            Ok(Self::Bin { mime_type, content })
        } else {
            let path = path::Path::new(uri);
            let ext = utils::extension_of(path).unwrap();

            let capacity = base_id.len() + uri.len();
            let mut id = String::with_capacity(capacity);

            id.push_str(base_id);
            id.push('.');

            let mut components = path.components().peekable();

            while let Some(comp) = components.next() {
                match comp {
                    path::Component::Normal(comp) => {
                        let comp = match components.peek() {
                            Some(_) => comp,
                            None => path::Path::new(comp).file_stem().unwrap_or(comp),
                        };
                        id.push_str(comp.to_str().unwrap());
                    }
                    path::Component::CurDir => (),
                    _ => return Err(format!("unsupported path component: {comp:?}").into()),
                }
            }

            Ok(Self::File { id, ext })
        }
    }
}

fn load_buffer(
    cache: AnyCache,
    base_id: &str,
    buffer: gltf::Buffer,
    blob: &mut Option<Vec<u8>>,
) -> Result<Vec<u8>, BoxedError> {
    Ok(match buffer.source() {
        gltf::buffer::Source::Bin => blob.take().ok_or("missing binary portion of binary glTF")?,
        gltf::buffer::Source::Uri(uri) => match UriContent::parse_uri(base_id, uri, None)? {
            UriContent::Bin { content: data, .. } => data,
            UriContent::File { id, .. } => cache.load::<Bin>(&id)?.cloned().0,
        },
    })
}

fn load_image_from_buffer(
    buffer: &[u8],
    mime_type: Option<&str>,
) -> Result<image::DynamicImage, BoxedError> {
    let format = match mime_type {
        Some("image/png") => Some(image::ImageFormat::Png),
        Some("image/jpeg") => Some(image::ImageFormat::Jpeg),
        _ => None,
    };

    Ok(match format {
        Some(format) => image::load_from_memory_with_format(buffer, format),
        None => image::load_from_memory(buffer),
    }?)
}

fn load_image(
    cache: AnyCache,
    base_id: &str,
    buffers: &[Vec<u8>],
    image: gltf::Image,
) -> Result<image::DynamicImage, BoxedError> {
    match image.source() {
        gltf::image::Source::Uri { uri, mime_type } => {
            match UriContent::parse_uri(base_id, uri, mime_type)? {
                UriContent::Bin { content, mime_type } => {
                    load_image_from_buffer(&content, mime_type)
                }
                UriContent::File { id, ext } => match ext {
                    "png" => Ok(cache.load::<super::Png>(&id)?.cloned().0),
                    "jpeg" | "jpg" => Ok(cache.load::<super::Jpeg>(&id)?.cloned().0),
                    _ => Err("Unknown image type".into()),
                },
            }
        }
        gltf::image::Source::View { view, mime_type } => {
            let buffer = &buffers[view.buffer().index()];
            let offset = view.offset();
            let buffer = &buffer[offset..offset + view.length()];

            load_image_from_buffer(buffer, Some(mime_type))
        }
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "gltf")))]
impl Compound for Gltf {
    fn load(cache: AnyCache, id: &SharedString) -> Result<Self, BoxedError> {
        let gltf::Gltf { document, mut blob } = cache.load::<gltf::Gltf>(id)?.cloned();

        let base_id = match id.rfind('.') {
            Some(index) => &id[..index],
            None => "",
        };

        let buffers: Vec<_> = document
            .buffers()
            .map(|b| load_buffer(cache, base_id, b, &mut blob))
            .collect::<Result<_, _>>()?;
        let images = document
            .images()
            .map(|i| load_image(cache, base_id, &buffers, i))
            .collect::<Result<_, _>>()?;

        Ok(Gltf {
            document,
            images,
            buffers,
        })
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "gltf")))]
impl super::DirLoadable for Gltf {
    fn select_ids(cache: AnyCache, id: &SharedString) -> std::io::Result<Vec<SharedString>> {
        gltf::Gltf::select_ids(cache, id)
    }
}
