use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use syn::parse::{Parse, ParseStream};


pub struct Input(PathBuf);

impl Parse for Input {
    fn parse(input: ParseStream) -> Result<Self, syn::Error> {
        let lit_path = input.parse::<syn::LitStr>()?;

        match Path::new(&lit_path.value()).canonicalize() {
            Ok(path) => Ok(Input(path)),
            Err(e) => Err(syn::Error::new(lit_path.span(), e))
        }
    }
}

impl Input {
    pub fn expand_dir(&self) -> Result<TokenStream, Vec<syn::Error>> {
        let mut errors = Vec::new();
        let mut content = Content::new();
        content.push_dir(Id::new());

        read_dir(&self.0, &mut content, Id::new(), &mut errors);

        if errors.is_empty() {
            Ok(content.to_token_stream())
        } else {
            Err(errors)
        }
    }
}


fn extension_of(path: &Path) -> Option<&str> {
    match path.extension() {
        Some(ext) => ext.to_str(),
        None => Some(""),
    }
}

fn push_error<T: std::fmt::Display>(errors: &mut Vec<syn::Error>, err: T) {
    errors.push(syn::Error::new(Span::call_site(), err));
}

fn read_dir(path: &Path, content: &mut Content, id: Id, errors: &mut Vec<syn::Error>) {
    let dir = match path.read_dir() {
        Ok(dir) => dir,
        Err(e) => {
            push_error(errors, format!("{}: {}", path.display(), e));
            return;
        }
    };

    for elem in dir {
        let path = match elem {
            Ok(e) => e.path(),
            Err(e) => {
                push_error(errors, format!("{}: {}", path.display(), e));
                continue;
            }
        };

        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            let this_id = id.clone().push(stem);

            if path.is_dir() {
                content.push_dir(this_id.clone());
                read_dir(&path, content, this_id, errors);
            } else if path.is_file() {
                if let Some(ext) = extension_of(&path) {
                    let ext = ext.to_owned();
                    let stem = stem.to_owned();
                    let desc = FileDesc(this_id, ext, path);
                    content.push_file(desc, stem, &id);
                }
            }
        }
    }
}


#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Id(String);

impl Id {
    fn new() -> Id {
        Id(String::new())
    }

    fn push(mut self, id: &str) -> Id {
        if !self.0.is_empty() {
            self.0.push('.');
        }
        self.0.push_str(id);
        self
    }
}


struct FileDesc(Id, String, PathBuf);

struct Content {
    files: Vec<FileDesc>,
    dirs: HashMap<Id, Vec<(String, String)>>,
}

impl Content {
    fn new() -> Content {
        Content {
            files: Vec::new(),
            dirs: HashMap::new(),
        }
    }

    fn push_file(&mut self, desc: FileDesc, stem: String, dir_id: &Id) {
        self.dirs.get_mut(dir_id).expect("File without directory").push((stem, desc.1.clone()));
        self.files.push(desc);
    }

    fn push_dir(&mut self, id: Id) {
        self.dirs.insert(id, Vec::new());
    }

    fn to_token_stream(&self) -> TokenStream {
        let files = self.files.iter().map(|FileDesc(Id(id), ext, path)| {
            let path = path.display().to_string();
            quote! {
                ((#id, #ext), (include_bytes!(#path) as &[u8]))
            }
        });

        let dirs = self.dirs.iter().map(|(Id(id), files)| {
            let files = files.iter().map(|(id, ext)| quote!{ (#id, #ext) });
            quote! {
                (#id, &[ #(#files),* ] as &[(&str, &str)])
            }
        });

        quote! {
            assets_manager::source::RawEmbedded {
                files: &[
                    #(#files),*
                ],
                dirs: &[
                    #(#dirs),*
                ],
            }
        }
    }
}
