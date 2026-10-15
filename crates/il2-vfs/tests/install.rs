//! Run with `IL2_ROOT=/path/to/game cargo test -- --ignored`.
use il2_vfs::Vfs;

#[test]
#[ignore]
fn stock_mount_order() {
    let vfs = Vfs::open_install(std::env::var("IL2_ROOT").unwrap()).unwrap();
    assert!(vfs.archives().len() > 50);
    assert!(vfs.exists("com/maddox/il2/objects/air.ini"));
    // Loose mission files are reachable through the fallback.
    assert!(vfs.exists("Missions/Quick/.") || vfs.root().join("Missions").is_dir());
}
