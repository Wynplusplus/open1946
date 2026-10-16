//! Track recording and playback in OpenIL2's own format (`docs/formats/track.md`): the options the
//! flight was started with, then state frames at 20 Hz with the events between them.

use glam::{DQuat, DVec3, Quat, Vec3};
use std::io::{Read, Write};

pub const MAGIC: &[u8; 8] = b"OIL2TRK1";
/// Seconds between frames.
pub const STEP: f64 = 0.05;

#[derive(Clone, Debug, Default)]
pub struct AircraftState {
    pub id: u32,
    pub pos: DVec3,
    pub ori: Quat,
    pub vel: Vec3,
    /// aileron, elevator, rudder, flaps, gear, throttle, air brake.
    pub ct: [f32; 7],
    pub destroyed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct UnitState {
    pub pos: Vec3,
    pub yaw: f32,
    pub alive: bool,
}

#[derive(Clone, Debug, Default)]
pub struct BulletSpawn {
    pub pos: DVec3,
    pub vel: Vec3,
    pub gun: String,
    pub kind: u8,
    pub tracer: bool,
    pub life: f32,
    pub fuse: Option<f32>,
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub t: f64,
    pub aircraft: Vec<AircraftState>,
    pub units: Vec<UnitState>,
    /// (aircraft id, chunk, levels).
    pub damage: Vec<(u32, String, u8)>,
    /// Map objects destroyed.
    pub kills: Vec<u32>,
    /// Explosions (point, kg TNT).
    pub booms: Vec<(DVec3, f32)>,
    pub bullets: Vec<BulletSpawn>,
}

struct W<'a>(&'a mut Vec<u8>);

impl W<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: usize) {
        self.0.extend((v.min(65535) as u16).to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend(v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.0.extend(v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u16(s.len());
        self.0.extend(&s.as_bytes()[..s.len().min(65535)]);
    }
    fn dvec3(&mut self, v: DVec3) {
        self.f64(v.x);
        self.f64(v.y);
        self.f64(v.z);
    }
    fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
}

struct R<'a> {
    d: &'a [u8],
    p: usize,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.d.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<usize> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?) as usize)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u16()?;
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn dvec3(&mut self) -> Option<DVec3> {
        Some(DVec3::new(self.f64()?, self.f64()?, self.f64()?))
    }
    fn vec3(&mut self) -> Option<Vec3> {
        Some(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
}

/// Writes `options` (the command line that sets the flight up) and then frames.
pub struct Recorder {
    file: std::io::BufWriter<std::fs::File>,
    pub next_t: f64,
    pub last_bullet: u64,
    pub last_kill: usize,
}

impl Recorder {
    pub fn create(path: &std::path::Path, options: &[String]) -> std::io::Result<Recorder> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut buf = Vec::new();
        let mut w = W(&mut buf);
        w.u16(options.len());
        for o in options {
            w.str(o);
        }
        file.write_all(MAGIC)?;
        file.write_all(&buf)?;
        Ok(Recorder {
            file,
            next_t: 0.0,
            last_bullet: 0,
            last_kill: 0,
        })
    }

    pub fn write(&mut self, f: &Frame) {
        let mut buf = Vec::new();
        let mut w = W(&mut buf);
        w.u8(1);
        w.f64(f.t);
        w.u16(f.aircraft.len());
        for a in &f.aircraft {
            w.u32(a.id);
            w.dvec3(a.pos);
            for c in a.ori.to_array() {
                w.f32(c);
            }
            w.vec3(a.vel);
            for c in a.ct {
                w.f32(c);
            }
            w.u8(a.destroyed as u8);
        }
        w.u32(f.units.len() as u32);
        for u in &f.units {
            w.vec3(u.pos);
            w.f32(u.yaw);
            w.u8(u.alive as u8);
        }
        w.u16(f.damage.len());
        for (id, chunk, n) in &f.damage {
            w.u32(*id);
            w.str(chunk);
            w.u8(*n);
        }
        w.u32(f.kills.len() as u32);
        for k in &f.kills {
            w.u32(*k);
        }
        w.u16(f.booms.len());
        for (p, power) in &f.booms {
            w.dvec3(*p);
            w.f32(*power);
        }
        w.u32(f.bullets.len() as u32);
        for b in &f.bullets {
            w.dvec3(b.pos);
            w.vec3(b.vel);
            w.str(&b.gun);
            w.u8(b.kind);
            w.u8(b.tracer as u8);
            w.f32(b.life);
            w.f32(b.fuse.unwrap_or(-1.0));
        }
        let _ = self.file.write_all(&buf);
    }

    pub fn flush(&mut self) {
        let _ = self.file.flush();
    }
}

/// A whole track in memory.
pub struct Track {
    pub options: Vec<String>,
    pub frames: Vec<Frame>,
}

pub fn read(path: &std::path::Path) -> Result<Track, String> {
    let mut d = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut d))
        .map_err(|e| e.to_string())?;
    if d.get(..8) != Some(MAGIC) {
        return Err("not an OpenIL2 track".into());
    }
    let mut r = R { d: &d, p: 8 };
    let n = r.u16().ok_or("truncated")?;
    let options = (0..n)
        .map(|_| r.str())
        .collect::<Option<Vec<_>>>()
        .ok_or("truncated")?;
    let mut frames = Vec::new();
    while r.p < d.len() {
        let Some(f) = frame(&mut r) else { break };
        frames.push(f);
    }
    Ok(Track { options, frames })
}

fn frame(r: &mut R) -> Option<Frame> {
    if r.u8()? != 1 {
        return None;
    }
    let mut f = Frame {
        t: r.f64()?,
        ..Default::default()
    };
    for _ in 0..r.u16()? {
        let id = r.u32()?;
        let pos = r.dvec3()?;
        let ori = Quat::from_array([r.f32()?, r.f32()?, r.f32()?, r.f32()?]);
        let vel = r.vec3()?;
        let mut ct = [0f32; 7];
        for c in &mut ct {
            *c = r.f32()?;
        }
        f.aircraft.push(AircraftState {
            id,
            pos,
            ori,
            vel,
            ct,
            destroyed: r.u8()? != 0,
        });
    }
    for _ in 0..r.u32()? {
        f.units.push(UnitState {
            pos: r.vec3()?,
            yaw: r.f32()?,
            alive: r.u8()? != 0,
        });
    }
    for _ in 0..r.u16()? {
        f.damage.push((r.u32()?, r.str()?, r.u8()?));
    }
    for _ in 0..r.u32()? {
        f.kills.push(r.u32()?);
    }
    for _ in 0..r.u16()? {
        f.booms.push((r.dvec3()?, r.f32()?));
    }
    for _ in 0..r.u32()? {
        let (pos, vel, gun, kind, tracer, life, fuse) = (
            r.dvec3()?,
            r.vec3()?,
            r.str()?,
            r.u8()?,
            r.u8()? != 0,
            r.f32()?,
            r.f32()?,
        );
        f.bullets.push(BulletSpawn {
            pos,
            vel,
            gun,
            kind,
            tracer,
            life,
            fuse: (fuse >= 0.0).then_some(fuse),
        });
    }
    Some(f)
}

/// Interpolates an aircraft between two frames.
pub fn lerp(a: &AircraftState, b: &AircraftState, t: f32) -> (DVec3, DQuat) {
    (
        a.pos.lerp(b.pos, t as f64),
        a.ori.slerp(b.ori, t).as_dquat(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let path = std::env::temp_dir().join("openil2_track_test.otrk");
        let mut rec = Recorder::create(&path, &["--mission".into(), "x.mis".into()]).unwrap();
        let f = Frame {
            t: 1.5,
            aircraft: vec![AircraftState {
                id: 3,
                pos: DVec3::new(1.0, 2.0, 3.0),
                ori: Quat::IDENTITY,
                ..Default::default()
            }],
            damage: vec![(3, "WingLIn_D0".into(), 2)],
            bullets: vec![BulletSpawn {
                gun: "MGunMG131si".into(),
                fuse: Some(2.0),
                ..Default::default()
            }],
            ..Default::default()
        };
        rec.write(&f);
        rec.flush();
        drop(rec);
        let t = read(&path).unwrap();
        assert_eq!(t.options, vec!["--mission", "x.mis"]);
        assert_eq!(t.frames.len(), 1);
        assert_eq!(t.frames[0].aircraft[0].pos, DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(t.frames[0].damage[0].1, "WingLIn_D0");
        assert_eq!(t.frames[0].bullets[0].fuse, Some(2.0));
    }
}
