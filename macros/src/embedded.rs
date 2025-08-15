use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use syn::parse::{Parse, ParseStream};

pub struct Input(PathBuf);

impl Parse for Input {
    fn parse(input: ParseStream) -> Result<Self, syn::Error> {
        let lit_path = input.parse::<syn::LitStr>()?;

        match Path::new(&lit_path.value()).canonicalize() {
            Ok(path) => Ok(Input(path)),
            Err(e) => Err(syn::Error::new(lit_path.span(), e)),
        }
    }
}

impl Input {
    pub fn expand_dir(&self) -> Result<TokenStream, Vec<syn::Error>> {
        let mut errors = Vec::new();
        let mut content = Content::new();
        content.push_dir(None, Id::new());

        read_dir(&self.0, &mut content, Id::new(), &mut errors);

        if errors.is_empty() {
            content.sort();
            Ok(content.to_token_stream())
        } else {
            Err(errors)
        }
    }
}

pub(crate) fn split_file_name(path: &Path) -> Option<(&str, &str)> {
    let name = path.file_name()?.to_str()?;
    match name.split_once('.') {
        Some(("", _)) => None,
        Some(res) => Some(res),
        None => Some((name, "")),
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

        let Some((name, ext)) = split_file_name(&path) else {
            continue;
        };

        let this_id = id.clone().push(name);

        let Ok(meta) = path.metadata() else {
            continue;
        };

        if meta.is_dir() {
            content.push_dir(Some(&id), this_id.clone());
            read_dir(&path, content, this_id, errors);
        } else if meta.is_file() {
            let desc = FileDesc(this_id, ext.to_owned(), path);
            content.push_file(desc, &id);
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum DirEntry {
    File(Id, String),
    Dir(Id),
}

struct Content {
    files: Vec<FileDesc>,
    dirs: BTreeMap<Id, Vec<DirEntry>>,
}

impl Content {
    fn new() -> Content {
        Content {
            files: Vec::new(),
            dirs: BTreeMap::new(),
        }
    }

    fn push_file(&mut self, desc: FileDesc, dir_id: &Id) {
        let entry = DirEntry::File(desc.0.clone(), desc.1.clone());
        self.dirs
            .get_mut(dir_id)
            .expect("File without directory")
            .push(entry);
        self.files.push(desc);
    }

    fn push_dir(&mut self, parent: Option<&Id>, id: Id) {
        if let Some(parent) = parent {
            let entry = DirEntry::Dir(id.clone());
            self.dirs
                .get_mut(parent)
                .expect("Directory without parent")
                .push(entry);
        }
        self.dirs.insert(id, Vec::new());
    }

    /// Sorts directory content to ensure reproducible builds.
    fn sort(&mut self) {
        // We can't use `sort_unstable_by_key` for some lifetime reason.
        self.files
            .sort_unstable_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));

        for dir in self.dirs.values_mut() {
            dir.sort_unstable();
        }
    }

    fn to_token_stream(&self) -> TokenStream {
        let files = self.files.iter().map(|FileDesc(Id(id), ext, path)| {
            let path = path.display().to_string();
            quote! {
                ((#id, #ext), (include_bytes!(#path) as &[u8]))
            }
        });

        let dirs = self.dirs.iter().map(|(Id(id), entries)| {
            let entries = entries.iter().map(|entry| match entry {
                DirEntry::File(Id(id), ext) => quote! {
                    assets_manager::source::DirEntry::File(#id, #ext)
                },
                DirEntry::Dir(Id(id)) => quote! {
                    assets_manager::source::DirEntry::Directory(#id)
                },
            });
            quote! {
                (#id, &[ #(#entries),* ])
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
