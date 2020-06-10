use crate::tests::X;
use std::{borrow::Cow, io};
use super::*;


fn raw(s: &str) -> io::Result<Cow<[u8]>> {
    Ok(s.as_bytes().into())
}

#[test]
fn string_loader_ok() {
    let raw = raw("Hello World!");
    let loaded = StringLoader::load(raw, "").unwrap();

    assert_eq!(loaded, "Hello World!");
}

#[test]
fn string_loader_utf8_err() {
    let raw = Ok(b"e\xa2"[..].into());
    assert!(StringLoader::load(raw, "").is_err());
}

#[test]
fn string_loader_io_err() {
    let err = Err(io::Error::last_os_error());
    assert!(StringLoader::load(err, "").is_err());
}

#[test]
fn bytes_loader_ok() {
    let raw = Ok(b"Hello World!"[..].into());
    let loaded = BytesLoader::load(raw, "").unwrap();

    assert_eq!(loaded, b"Hello World!");
}

#[test]
fn bytes_loader_io_err() {
    let err = Err(io::Error::last_os_error());
    assert!(BytesLoader::load(err, "").is_err());
}

#[test]
fn load_or_default_ok() {
    let n = rand::random::<i32>();
    let s = &format!("{}", n);
    let raw = raw(s);

    let loaded: i32 = LoadOrDefault::<ParseLoader>::load(raw, "").unwrap();
    assert_eq!(loaded, n);
}

#[test]
fn load_or_default_err() {
    let raw = raw("a");
    let loaded: i32 = LoadOrDefault::<ParseLoader>::load(raw, "").unwrap();
    assert_eq!(loaded, 0);
}

#[test]
fn parse_loader_ok() {
    let n = rand::random::<i32>();
    let s = &format!("{}", n);
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
fn parse_loader_io_err() {
    let err = Err(io::Error::last_os_error());
    let res: Result<i32, _> = ParseLoader::load(err, "");
    assert!(res.is_err());
}

#[test]
fn from_other() {
    let n = rand::random::<i32>();
    let s = &format!("{}", n);
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
                let raw = Ok(($ser)(&point).unwrap().into());

                let loaded: Point = <$loader>::load(raw, "").unwrap();

                assert_eq!(loaded, point);
            }

            #[test]
            fn $name_err() {
                let err = Err(io::Error::last_os_error());
                let loaded: Result<Point, _> = <$loader>::load(err, "");

                assert!(loaded.is_err());
            }
        }
    }
}}

#[cfg(feature = "bincode")]
test_loader!(bincode_loader_ok, bincode_loader_err, BincodeLoader, serde_bincode::serialize);

#[cfg(feature = "cbor")]
test_loader!(cbor_loader_ok, cbor_loader_err, CborLoader, serde_cbor::to_vec);

#[cfg(feature = "json")]
test_loader!(json_loader_ok, json_loader_err, JsonLoader, serde_json::to_vec);

#[cfg(feature = "msgpack")]
test_loader!(msgpack_loader_ok, msgpack_err, MessagePackLoader, serde_msgpack::encode::to_vec);

#[cfg(feature = "ron")]
test_loader!(ron_loader_ok, ron_loader_err, RonLoader, |p| serde_ron::ser::to_string(p).map(String::into_bytes));

#[cfg(feature = "toml")]
test_loader!(toml_loader_ok, toml_loader_err, TomlLoader, serde_toml::ser::to_vec);

#[cfg(feature = "yaml")]
test_loader!(yaml_loader_ok, yaml_loader_err, YamlLoader, serde_yaml::to_vec);
