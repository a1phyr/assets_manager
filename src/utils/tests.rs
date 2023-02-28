use super::*;

mod shared_bytes {
    use super::SharedBytes;

    #[test]
    fn slice() {
        let bytes = SharedBytes::from(&b"test"[..]);
        assert_eq!(&*bytes, b"test");
        let b2 = bytes.clone();
        assert_eq!(&*b2, b"test");
        assert_eq!(&*bytes, b"test");
        drop(bytes);
        assert_eq!(&*b2, b"test");
    }

    #[test]
    fn vec() {
        let bytes = SharedBytes::from(Vec::from(&b"test"[..]));
        assert_eq!(&*bytes, b"test");
        let b2 = bytes.clone();
        assert_eq!(&*b2, b"test");
        assert_eq!(&*bytes, b"test");
        drop(bytes);
        assert_eq!(&*b2, b"test");
    }
}

#[cfg(feature = "utils")]
mod cell {
    use crate::OnceInitCell;

    #[test]
    fn well_init() {
        let cell = OnceInitCell::<(), ()>::with_value(());
        assert!(cell.get().is_some());

        let cell = OnceInitCell::new("test".to_owned());

        // Errors left the cell initialized
        let res = cell.get_or_try_init(|_| Err(()));
        assert!(res.is_err());
        assert!(cell.get().is_none());

        // Panics are well supported
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cell.get_or_try_init(|_| -> Result<_, ()> { panic!() })
        }));
        assert!(res.is_err());
        assert!(cell.get().is_none());

        // Valid path
        let res = cell.get_or_init(|s| s.to_owned() + " ok");
        assert_eq!(res, "test ok");
        assert_eq!(cell.get(), Some(&"test ok".to_owned()));

        // No double-inits
        cell.get_or_init(|_| panic!("multiple init"));
    }

    #[test]
    #[should_panic]
    fn drop_bomb() {
        struct Bomb;
        impl Drop for Bomb {
            fn drop(&mut self) {
                panic!("bomb");
            }
        }

        let cell = OnceInitCell::new(Bomb);
        cell.get_or_init(|_| ());
    }
}
