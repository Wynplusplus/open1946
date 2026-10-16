//! Prints a mesh's LODs and materials: `mshinfo <vfs path>`.
fn main() {
    let path = std::env::args().nth(1).expect("usage: mshinfo <path.msh>");
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let mesh = il2_formats::msh::Mesh::parse(&vfs.read(&path).unwrap()).unwrap();
    for (i, lod) in mesh.lods.iter().enumerate() {
        println!(
            "lod {i}: {} faces, materials {:?}",
            lod.faces.len(),
            lod.materials
        );
    }
}
