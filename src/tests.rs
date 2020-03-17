mod cache_entry {
    use std::sync::Mutex;
    use crate::lock::CacheEntry;

    struct DropCounter<'a> {
        count: &'a Mutex<usize>,
    }

    impl Drop for DropCounter<'_> {
        fn drop(&mut self) {
            let mut count = self.count.lock().unwrap();
            *count += 1;
        }
    }

    #[test]
    fn drop_inner() {
        let count = &Mutex::new(0);

        let entry_1 = CacheEntry::new(DropCounter { count });
        let entry_2 = CacheEntry::new(DropCounter { count });
        assert_eq!(*count.lock().unwrap(), 0);
        drop(entry_1);
        assert_eq!(*count.lock().unwrap(), 1);
        drop(entry_2);
        assert_eq!(*count.lock().unwrap(), 2);
    }

    #[test]
    fn read() {
        let val = rand::random::<i32>();

        let entry = CacheEntry::new(val);
        let guard = unsafe { entry.get_ref::<i32>() };

        assert_eq!(*guard.read(), val);
    }

    #[test]
    fn write() {
        let x = rand::random::<i32>();
        let y = rand::random::<i32>();

        let entry = CacheEntry::new(x);
        unsafe {
            let guard = entry.write(y);
            assert_eq!(*guard.read(), y);
            let guard = entry.get_ref::<i32>();
            assert_eq!(*guard.read(), y);
        }
    }

    #[test]
    fn ptr_eq() {
        let x = rand::random::<i32>();

        let entry = CacheEntry::new(x);
        unsafe {
            let ref_1 = entry.get_ref::<i32>();
            let ref_2 = entry.get_ref::<i32>();
            assert!(ref_1.ptr_eq(&ref_2));
        }
    }
}
