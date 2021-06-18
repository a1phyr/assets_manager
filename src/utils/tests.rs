use super::*;

mod shared_bytes {
    use super::SharedBytes;

    #[test]
    fn slice() {
        let bytes = SharedBytes::from(&b"test"[..]);
        assert_eq!(&*bytes, b"test");
        let b2 = bytes.clone();
        assert_eq!(&*b2, b"test");
        drop(b2);
        assert_eq!(&*bytes, b"test");
    }

    #[test]
    fn vec() {
        let bytes = SharedBytes::from(Vec::from(&b"test"[..]));
        assert_eq!(&*bytes, b"test");
        let b2 = bytes.clone();
        assert_eq!(&*b2, b"test");
        drop(b2);
        assert_eq!(&*bytes, b"test");
    }
}
