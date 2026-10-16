//! Lists bridges of a .static read from stdin.
use std::io::Read;
fn main() {
    let mut d = Vec::new();
    std::io::stdin().read_to_end(&mut d).unwrap();
    let s = il2_formats::statics::Statics::parse(&d).unwrap();
    for b in s.bridges.iter().take(8) {
        println!("{b:?}");
    }
    println!("{} bridges", s.bridges.len());
}
