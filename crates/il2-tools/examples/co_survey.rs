//! Survey of mesh collision sections: part types, blocks and frame counts over many meshes.
fn main() {
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").unwrap()).unwrap();
    let names = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let mut types = std::collections::BTreeMap::new();
    let mut blocks = std::collections::BTreeMap::new();
    let mut frames = std::collections::BTreeMap::new();
    let mut n = 0;
    for name in names
        .lines()
        .filter(|l| l.to_ascii_lowercase().ends_with(".msh"))
        .step_by(7)
        .take(3000)
    {
        let Ok(d) = vfs.read(name) else { continue };
        let Ok(f) = il2_formats::binsect::RowFile::parse(&d) else {
            continue;
        };
        n += 1;
        if let Some(c) = f.section("CoCommon") {
            let nb = c
                .rows
                .first()
                .and_then(|r| r.get(1))
                .and_then(|v| v.as_i32())
                .unwrap_or(-1);
            *blocks.entry(nb).or_insert(0) += 1;
        }
        for s in &f.sections {
            if s.name.starts_with("CoCommon_b") && s.name.contains('p') {
                for r in &s.rows {
                    let k = r.first().and_then(|v| v.as_str()).unwrap_or("");
                    let v = format!("{:?}", r.get(1));
                    if k == "Type" {
                        *types.entry(v.clone()).or_insert(0) += 1;
                    }
                    if k == "NFrames" {
                        *frames.entry(v).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    println!("{n} meshes\ntypes {types:?}\nblocks {blocks:?}\nframes {frames:?}");
}
