//! Integration tests against a real game install. Run with
//! `IL2_ROOT=/path/to/game cargo test -- --ignored`.

use il2_sfs::Archive;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(std::env::var("IL2_ROOT").expect("set IL2_ROOT to the game directory"))
}

#[test]
#[ignore]
fn files_sfs_known_entries() {
    let a = Archive::open(root().join("files.SFS")).unwrap();
    let e = a.find("com/maddox/il2/objects/air.ini").expect("air.ini present");
    let data = a.read(e).unwrap();
    assert!(!data.is_empty());
    assert!(data.iter().all(|&b| b.is_ascii()), "air.ini is plain text");
}
