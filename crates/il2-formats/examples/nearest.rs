//! Nearest runway ends to a point: `nearest x y < file.static`.
use std::io::Read;
fn main() {
    let a: Vec<f32> = std::env::args()
        .skip(1)
        .map(|s| s.parse().unwrap())
        .collect();
    let mut d = Vec::new();
    std::io::stdin().read_to_end(&mut d).unwrap();
    let s = il2_formats::statics::Statics::parse(&d).unwrap();
    let mut v: Vec<_> = s
        .runway_ends()
        .map(|e| ((e.x - a[0]).hypot(e.y - a[1]), e.clone()))
        .collect();
    v.sort_by(|x, y| x.0.total_cmp(&y.0));
    for (dist, e) in v.iter().take(4) {
        println!(
            "{dist:8.0} m  ({:.0}, {:.0}) yaw {:.0}",
            e.x,
            e.y,
            e.yaw.rem_euclid(360.0)
        );
    }
}
