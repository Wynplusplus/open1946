//! Tests runway pairing under candidate yaw conventions for a .static file read from stdin.
use std::io::Read;
fn main() {
    let mut d = Vec::new();
    std::io::stdin().read_to_end(&mut d).unwrap();
    let s = il2_formats::statics::Statics::parse(&d).unwrap();
    let ends: Vec<_> = s.runway_ends().collect();
    for (name, f) in [
        ("cos,sin", (|y: f32| (y.cos(), y.sin())) as fn(f32) -> (f32, f32)),
        ("cos,-sin", |y: f32| (y.cos(), -y.sin())),
        ("sin,cos", |y: f32| (y.sin(), y.cos())),
    ] {
        let mut paired = 0;
        for a in &ends {
            let (c, sn) = f(a.yaw.rem_euclid(360.0).to_radians());
            let hit = ends.iter().any(|b| {
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let along = dx * c + dy * sn;
                let side = -dx * sn + dy * c;
                let (bc, bs) = f(b.yaw.rem_euclid(360.0).to_radians());
                side.abs() < 15.0 && along < -800.0 && along > -2500.0 && bc * c + bs * sn < -0.9
            });
            paired += hit as usize;
        }
        println!("{name:8}: {paired} of {} ends have an opposite end", ends.len());
    }
}
