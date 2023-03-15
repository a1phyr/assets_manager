use super::*;

macro_rules! test_source {
    ($source:expr) => {
        #[test]
        fn read_ok() {
            let source = $source;
            let content = source.read("test.b", "x").unwrap();
            assert_eq!(content.as_ref(), &*b"-7");
        }

        #[test]
        fn read_err() {
            let source = $source;
            assert!(source.read("test.not_found", "x").is_err());
        }

        #[test]
        fn read_dir() {
            let source = $source;
            let mut dirs = Vec::new();
            let mut files = Vec::new();

            source
                .read_dir("test.read_dir", &mut |entry| match entry {
                    DirEntry::File(id, ext) => files.push((String::from(id), String::from(ext))),
                    DirEntry::Directory(id) => dirs.push(id.to_owned()),
                })
                .unwrap();

            dirs.sort();
            files.sort();
            assert_eq!(
                files,
                [
                    (String::from("test.read_dir.c"), String::from("txt")),
                    (String::from("test.read_dir.d"), String::from("")),
                ]
            );
            assert_eq!(dirs, ["test.read_dir.a", "test.read_dir.b"]);
        }

        #[test]
        fn read_root() {
            let source = $source;
            let mut dir = Vec::new();

            source
                .read_dir("", &mut |entry| {
                    if let DirEntry::Directory(id) = entry {
                        dir.push(id.to_owned());
                    }
                })
                .unwrap();

            dir.sort();
            assert_eq!(dir, ["common", "example", "test"]);
        }
    };
}

mod filesystem {
    use super::*;

    test_source!(FileSystem::new("assets").unwrap());

    #[test]
    fn path_of() {
        let fs = FileSystem::new("assets").unwrap();

        // Necessary because of `canonicalize`
        let path = {
            let mut path = fs.root().to_owned();
            path.extend(["test", "a"]);
            path.set_extension("x");
            path
        };

        assert_eq!(path, fs.path_of(DirEntry::File("test.a", "x")));
    }
}

#[cfg(feature = "embedded")]
mod embedded {
    use super::*;

    static RAW: RawEmbedded<'static> = embed!("assets");

    test_source!(Embedded::from(RAW));
}

#[cfg(feature = "zip-deflate")]
mod zip {
    use super::*;

    test_source!(Zip::open("assets/test/test.zip").unwrap());
}
