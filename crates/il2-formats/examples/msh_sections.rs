//! Prints a mesh file's sections (names, row counts and the first rows); the collision sections in full.
fn main() {
    let path = std::env::args().nth(1).expect("path to a .msh file");
    let data = std::fs::read(path).unwrap();
    let f = il2_formats::binsect::RowFile::parse(&data).unwrap();
    for s in &f.sections {
        let co = s.name.starts_with("Co");
        println!("[{}] {} rows", s.name, s.rows.len());
        for r in s.rows.iter().take(if co { 12 } else { 2 }) {
            println!("  {:?}", r);
        }
    }
}
