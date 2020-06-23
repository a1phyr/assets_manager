use super::*;

#[test]
fn source_object_safe() {
    let s = FileSystem::new(".").unwrap();
    let _: &dyn Source = &Box::new(s);
}

mod filesystem {
    use super::*;

    #[test]
    fn read_ok() {
        let fs = FileSystem::new("assets").unwrap();
        let content = fs.read("test.b", "x").unwrap();
        assert_eq!(&*content, &*b"-7");
    }

    #[test]
    fn read_err() {
        let fs = FileSystem::new("assets").unwrap();
        assert!(fs.read("test.not_found", "x").is_err());
    }

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

    #[test]
    fn read_dir() {
        let fs = FileSystem::new("assets").unwrap();
        
        let mut dir = fs.read_dir("test", &["x"]).unwrap();
        dir.sort();
        assert_eq!(dir, ["a", "b", "cache"]);
    }
}
