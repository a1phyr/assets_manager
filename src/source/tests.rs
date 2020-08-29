use super::*;

#[test]
fn source_object_safe() {
    let s = FileSystem::new(".").unwrap();
    let _: &dyn Source = &Box::new(s);
}

macro_rules! test_source {
    ($source:expr) => {
        #[test]
        fn read_ok() {
            let source = $source;
            let content = source.read("test.b", "x").unwrap();
            assert_eq!(&*content, &*b"-7");
        }

        #[test]
        fn read_err() {
            let source = $source;
            assert!(source.read("test.not_found", "x").is_err());
        }

        #[test]
        fn read_dir() {
            let source = $source;

            let mut dir = source.read_dir("test", &["x"]).unwrap();
            dir.sort();
            assert_eq!(dir, ["a", "b", "cache"]);
        }
    }
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
            path.extend(&["test", "a"]);
            path.set_extension("x");
            path
        };

        assert_eq!(path, fs.path_of("test.a", "x"));
    }
}

#[cfg(feature = "embedded")]
mod embedded {
    use super::*;

    static RAW: RawEmbedded<'static> = embed!("assets");

    test_source!(Embedded::from(RAW));
}
