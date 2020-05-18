use std::{borrow::Cow, io::Result};
use super::*;

fn raw(s: &str) -> Result<Cow<[u8]>> {
    Ok(s.as_bytes().into())
}

#[test]
fn string_loader() {
    let raw = raw("Hello World!");
    let loaded = StringLoader::load(raw).unwrap();

    assert_eq!(loaded, "Hello World!");
}

#[test]
fn load_or_default() {
    let raw = raw("a");

    let loaded: i32 = LoadOrDefault::<ParseLoader>::load(raw).unwrap();

    assert_eq!(loaded, 0);
}

#[test]
fn parse_loader() {
    let n = rand::random::<i32>();
    let s = &format!("{}", n);
    let raw = raw(s);

    let loaded: i32 = ParseLoader::load(raw).unwrap();

    assert_eq!(loaded, n);
}

#[test]
fn from_other() {
    use crate::tests::X;

    let n = rand::random::<i32>();
    let s = &format!("{}", n);
    let raw = raw(s);

    let loaded: X = LoadFrom::<i32, ParseLoader>::load(raw).unwrap();

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
        ($name:ident, $loader:ty, $ser:expr) => {
            #[test]
            fn $name() {
                let point = rand::random::<Point>();
                let raw = Ok(($ser)(&point).unwrap().into());

                let loaded: Point = <$loader>::load(raw).unwrap();

                assert_eq!(loaded, point);
            }
        }
    }
}}

#[cfg(feature = "bincode")]
test_loader!(bincode_loader, BincodeLoader, serde_bincode::serialize);

#[cfg(feature = "cbor")]
test_loader!(cbor_loader, CborLoader, serde_cbor::to_vec);

#[cfg(feature = "json")]
test_loader!(json_loader, JsonLoader, serde_json::to_vec);

#[cfg(feature = "msgpack")]
test_loader!(msgpack_loader, MessagePackLoader, serde_msgpack::encode::to_vec);

#[cfg(feature = "ron")]
test_loader!(ron_loader, RonLoader, |p| serde_ron::ser::to_string(p).map(String::into_bytes));

#[cfg(feature = "toml")]
test_loader!(toml_loader, TomlLoader, serde_toml::ser::to_vec);

#[cfg(feature = "yaml")]
test_loader!(yaml_loader, YamlLoader, serde_yaml::to_vec);
