//! Summarises the houses of a .static read from stdin.
use std::io::Read;
fn main() {
    let mut d = Vec::new();
    std::io::stdin().read_to_end(&mut d).unwrap();
    let s = il2_formats::statics::Statics::parse(&d).unwrap();
    let mut count = vec![0usize; s.house_classes.len()];
    for h in &s.houses {
        count[h.class as usize] += 1;
    }
    let mut order: Vec<usize> = (0..count.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(count[i]));
    for &i in order.iter().take(15) {
        println!("{:8} {}", count[i], s.house_classes[i]);
    }
    if let Some(want) = std::env::args().nth(1) {
        if let Some(h) = s.houses.iter().find(|h| s.house_classes[h.class as usize].contains(&want)) {
            println!("first {want}: x {:.0} y {:.0} yaw {:.1}", h.x, h.y, h.yaw);
        }
    }
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for h in &s.houses {
        lo = [lo[0].min(h.x), lo[1].min(h.y)];
        hi = [hi[0].max(h.x), hi[1].max(h.y)];
    }
    let mut tiles = std::collections::HashMap::new();
    for h in s.houses.iter().filter(|h| !s.house_classes[h.class as usize].contains("Tree")) {
        *tiles.entry(((h.x / 1000.0) as i32, (h.y / 1000.0) as i32)).or_insert(0usize) += 1;
    }
    if let Some(((tx, ty), n)) = tiles.into_iter().max_by_key(|&(_, n)| n) {
        println!("densest tile: {n} houses around x {} y {}", tx * 1000 + 500, ty * 1000 + 500);
    }
    println!("{} houses in {} classes, dead {}, extent {lo:?}..{hi:?}", s.houses.len(), s.house_classes.len(), s.houses.iter().filter(|h| h.dead).count());
}
