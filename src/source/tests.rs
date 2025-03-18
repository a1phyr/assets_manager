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
    use std::error::Error;

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

    #[test]
    fn errors() {
        let fs = FileSystem::new("assets").unwrap();

        let err = fs.read("file_name", "ext").unwrap_err();
        assert!(err.to_string().contains("file_name.ext"));
        assert!(err.kind() == io::ErrorKind::NotFound);

        let inner = err.source().unwrap().downcast_ref::<io::Error>().unwrap();
        assert!(inner.raw_os_error().is_some());
        assert!(err.kind() == io::ErrorKind::NotFound);
    }
}

#[cfg(feature = "embedded")]
mod embedded {
    use super::*;

    static RAW: RawEmbedded<'static> = embed!("assets");

    test_source!(Embedded::from(RAW));
}

#[cfg(feature = "tar")]
mod tar {
    use super::*;

    test_source!(Tar::open("assets/test/test.tar").unwrap());

    #[test]
    fn errors() {
        let tar = Tar::open("assets/test/test.tar").unwrap();

        let err = tar.read("file_name", "ext").unwrap_err();
        assert!(err.to_string().contains("file_name"));
        assert!(err.to_string().contains("assets/test/test.tar"));
        assert!(err.kind() == io::ErrorKind::NotFound);
    }

    #[test]
    fn direct_read() {
        let tar = std::fs::read("assets/test/test.tar").unwrap();
        let tar = Tar::from_bytes(tar).unwrap();

        let file = tar.read("test.b", "x").unwrap();
        assert!(matches!(file, FileContent::Slice(_)));
    }
}

#[cfg(feature = "zip-deflate")]
mod zip {
    use super::*;

    test_source!(Zip::open("assets/test/test.zip").unwrap());

    #[test]
    fn errors() {
        let zip = Zip::open("assets/test/test.zip").unwrap();

        let err = zip.read("file_name", "ext").unwrap_err();
        assert!(err.to_string().contains("file_name"));
        assert!(err.to_string().contains("assets/test/test.zip"));
        assert!(err.kind() == io::ErrorKind::NotFound);
    }
}
