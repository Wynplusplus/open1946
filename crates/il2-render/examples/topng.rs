//! Converts game images (IMF, TGA) from the VFS to PNG: `topng <vfs path> <out.png>`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let img = il2_render::scene::decode_image(&vfs.read(&a[1]).unwrap(), &a[1]).unwrap();
    println!("{} x {}", img.width, img.height);
    image::save_buffer(
        &a[2],
        &img.rgba,
        img.width,
        img.height,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
}
