use crate::{
    AssetCache,
    tests::X,
};
use std::{
    fs::{self, File},
    io::Write,
    thread,
    time::Duration,
};

fn sleep() {
    thread::sleep(Duration::from_millis(100));
}

type Res = Result<(), Box<dyn std::error::Error>>;

#[test]
fn reload_asset() -> Res {
    let cache = AssetCache::new("assets")?;
    let asset = cache.load::<X>("test.hot_asset.a")?;
    cache.hot_reload();

    let reload_and_test = |n, bytes| {
        File::create("assets/test/hot_asset/a.x")?.write_all(bytes)?;
        sleep();
        cache.hot_reload();
        assert_eq!(asset.read().0, n);
        Ok(())
    };

    reload_and_test(17, b"17")?;
    reload_and_test(42, b"42")
}

#[test]
fn dir_remove_and_add() -> Res {
    let cache = AssetCache::new("assets")?;
    let dir = cache.load_dir::<X>("test.hot_dir")?;
    cache.hot_reload();

    let assert_value = |t: &[i32]| {
        let res: Vec<_> = dir.iter().map(|x| x.read().0).collect();
        assert_eq!(res, t);
    };

    assert_value(&[61]);

    fs::remove_file("assets/test/hot_dir/a.x")?;
    sleep();
    cache.hot_reload();

    assert_value(&[]);

    File::create("assets/test/hot_dir/a.x")?.write_all(b"61")?;
    sleep();
    cache.hot_reload();

    assert_value(&[61]);

    Ok(())
}

#[test]
fn reload_static() -> Res {
    let cache = AssetCache::new("assets")?;
    let cache = Box::leak(Box::new(cache));
    let asset = cache.load::<X>("test.hot_asset.b")?;
    cache.enhance_hot_reloading();

    assert_eq!(asset.read().0, 22);
    File::create("assets/test/hot_asset/b.x")?.write_all(b"18")?;
    sleep();
    assert_eq!(asset.read().0, 18);
    File::create("assets/test/hot_asset/b.x")?.write_all(b"22")?;
    sleep();
    assert_eq!(asset.read().0, 22);

    Ok(())
}
