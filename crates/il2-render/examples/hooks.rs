//! Prints hook positions of a hierarchical mesh: `cargo run --example hooks -- <him>`.
fn main() {
    let path = std::env::args().nth(1).expect("usage: hooks <hier.him>");
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let loader = il2_render::scene::Loader::new(&vfs);
    for (name, p) in loader.hier_hooks(&path).unwrap() {
        println!("{name:<24} {:8.3} {:8.3} {:8.3}", p.x, p.y, p.z);
    }
}
