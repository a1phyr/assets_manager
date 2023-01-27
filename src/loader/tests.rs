use super::*;
use crate::tests::X;
use std::borrow::Cow;

fn raw(s: &str) -> Cow<[u8]> {
    s.as_bytes().into()
}

#[test]
fn string_loader_ok() {
    let raw = raw("Hello World!");

    let loaded: String = StringLoader::load(raw.clone(), "").unwrap();
    assert_eq!(loaded, "Hello World!");

    let loaded: Box<str> = StringLoader::load(raw, "").unwrap();
    assert_eq!(&*loaded, "Hello World!");
}

#[test]
fn string_loader_utf8_err() {
    let raw = b"e\xa2"[..].into();
    let result: Result<String, _> = StringLoader::load(raw, "");
    assert!(result.is_err());
}

#[test]
fn bytes_loader_ok() {
    let raw = raw("Hello World!");

    let loaded: Vec<u8> = BytesLoader::load(raw.clone(), "").unwrap();
    assert_eq!(loaded, b"Hello World!");

    let loaded: Box<[u8]> = BytesLoader::load(raw, "").unwrap();
    assert_eq!(&*loaded, b"Hello World!");
}

#[test]
fn parse_loader_ok() {
    let n = rand::random::<i32>();
    let s = &format!("{n}");
    let raw = raw(s);

    let loaded: i32 = ParseLoader::load(raw, "").unwrap();
    assert_eq!(loaded, n);
}

#[test]
fn parse_loader_err() {
    let raw = raw("x");
    let loaded: Result<i32, _> = ParseLoader::load(raw, "");
    assert!(loaded.is_err());
}

#[test]
fn from_other() {
    let n = rand::random::<i32>();
    let s = &format!("{n}");
    let raw = raw(s);

    let loaded: X = LoadFrom::<i32, ParseLoader>::load(raw, "").unwrap();

    assert_eq!(loaded, X(n));
}

cfg_if::cfg_if! { if #[cfg(feature = "serde")] {
    use serde::{Serialize, Deserialize};
    use rand::{
        Rng,
        distributions::{Distribution, Standard},
    };

    #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
    struct Point {
        x: i32,
        y: i32,
    }

    impl Distribution<Point> for Standard {
        fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Point {
            Point {
                x: rng.gen(),
                y: rng.gen(),
            }
        }
    }

    macro_rules! test_loader {
        ($name_ok:ident, $name_err:ident, $loader:ty, $ser:expr) => {
            #[test]
            fn $name_ok() {
                let point = rand::random::<Point>();
                let raw = ($ser)(&point).unwrap().into();

                let loaded: Point = <$loader>::load(Cow::Owned(raw), "").unwrap();

                assert_eq!(loaded, point);
            }

            #[test]
            fn $name_err() {
                let raw = raw("\x12ec\x4b".into());
                let loaded: Result<Point, _> = <$loader>::load(raw, "");

                assert!(loaded.is_err());
            }
        }
    }
}}

#[cfg(feature = "bincode")]
test_loader!(
    bincode_loader_ok,
    bincode_loader_err,
    BincodeLoader,
    bincode::serialize
);

#[cfg(feature = "json")]
test_loader!(
    json_loader_ok,
    json_loader_err,
    JsonLoader,
    serde_json::to_vec
);

#[cfg(feature = "msgpack")]
test_loader!(
    msgpack_loader_ok,
    msgpack_err,
    MessagePackLoader,
    rmp_serde::encode::to_vec
);

#[cfg(feature = "ron")]
test_loader!(
    ron_loader_ok,
    ron_loader_err,
    RonLoader,
    ron::ser::to_string
);

#[cfg(feature = "toml")]
test_loader!(
    toml_loader_ok,
    toml_loader_err,
    TomlLoader,
    toml_edit::ser::to_string
);

#[cfg(feature = "yaml")]
test_loader!(
    yaml_loader_ok,
    yaml_loader_err,
    YamlLoader,
    serde_yaml::to_string
);
