//! Map static object files (`maps/<map>/*.static`): `docs/formats/static.md`.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("static file: {0}")]
    Bad(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub class: usize,
    pub x: f32,
    pub y: f32,
    /// Degrees.
    pub yaw: f32,
}

/// A house or tree from the per-cell lists (`docs/formats/static.md`, section 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct House {
    /// Index into `Statics::house_classes`.
    pub class: u16,
    /// Starts destroyed.
    pub dead: bool,
    /// World metres.
    pub x: f64,
    pub y: f64,
    /// Degrees, counter-clockwise from east.
    pub yaw: f32,
}

impl House {
    /// Decodes one `(code, position)` record of the block at cell `(cx, cy)`.
    pub fn decode(cx: u16, cy: u16, code: i32, pos: i32) -> House {
        let scale = 200.0 / 32000.0;
        House {
            class: (code & 0x7fff) as u16,
            dead: code & 0x8000 != 0,
            x: cx as f64 * 200.0 + (pos as i16) as f64 * scale,
            y: cy as f64 * 200.0 + ((pos >> 16) as i16) as f64 * scale,
            yaw: ((code >> 16) as i16) as f32 * 360.0 / 32000.0,
        }
    }
}

/// A bridge between two land-type pixels (`docs/formats/static.md`, section 2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bridge {
    /// Land-type pixel column and row (row 0 at the north edge) of both ends.
    pub begin: (i32, i32),
    pub end: (i32, i32),
    /// The road bit it carries: 0x20 country road, 0x40 railway, 0x80 highway.
    pub kind: i32,
    /// Sideways shift of the segments, in pixels (|k| ≤ 0.4).
    pub offset: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Statics {
    pub bridges: Vec<Bridge>,
    pub classes: Vec<String>,
    pub objects: Vec<Placed>,
    pub house_classes: Vec<String>,
    pub houses: Vec<House>,
    /// Polylines of the airdrome layout.
    pub runways: Vec<Vec<[f32; 2]>>,
    pub taxiways: Vec<Vec<[f32; 2]>>,
    pub parking: Vec<Vec<[f32; 2]>>,
}

pub const RUNWAY_CLASS: &str = "com.maddox.il2.objects.air.Runaway";

struct R<'a> {
    d: &'a [u8],
    p: usize,
}

impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], Error> {
        let s = self
            .d
            .get(self.p..self.p + n)
            .ok_or(Error::Bad("truncated"))?;
        self.p += n;
        Ok(s)
    }
    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, Error> {
        let n = self.i32()?;
        usize::try_from(n)
            .ok()
            .filter(|&n| n < 10_000_000)
            .ok_or(Error::Bad("count"))
    }
    fn f32(&mut self) -> Result<f32, Error> {
        Ok(f32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn utf(&mut self) -> Result<String, Error> {
        let n = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn polylines(&mut self) -> Result<Vec<Vec<[f32; 2]>>, Error> {
        (0..self.count()?)
            .map(|_| {
                (0..self.count()?)
                    .map(|_| Ok([self.f32()?, self.f32()?]))
                    .collect()
            })
            .collect()
    }
}

impl Statics {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let mut r = R { d, p: 0 };
        if r.i32()? != -65535 {
            return Err(Error::Bad("version"));
        }
        let mut s = Statics::default();
        for _ in 0..r.count()? {
            let (x1, y1, x2, y2, kind) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?, r.i32()?);
            let offset = r.f32()?;
            s.bridges.push(Bridge {
                begin: (x1, y1),
                end: (x2, y2),
                kind,
                offset,
            });
        }
        let nclasses = r.count()?;
        if nclasses > 0 {
            s.classes = (0..nclasses).map(|_| r.utf()).collect::<Result<_, _>>()?;
            for _ in 0..r.count()? {
                let class = r.i32()?.max(0) as usize;
                let (x, y, yaw) = (r.f32()?, r.f32()?, r.f32()?);
                s.objects.push(Placed { class, x, y, yaw });
            }
        }
        let nhouse = r.count()?;
        if nhouse > 0 {
            s.house_classes = (0..nhouse).map(|_| r.utf()).collect::<Result<_, _>>()?;
            for _ in 0..r.count()? {
                let key = r.i32()?;
                let (cx, cy) = (key as u16, (key >> 16) as u16);
                for _ in 0..r.count()? {
                    let (code, pos) = (r.i32()?, r.i32()?);
                    let mut h = House::decode(cx, cy, code, pos);
                    // The runtime maps out-of-range class indices to class 0.
                    if h.class as usize >= nhouse {
                        h.class = 0;
                    }
                    s.houses.push(h);
                }
            }
        }
        if r.p < d.len() {
            s.runways = r.polylines()?;
            s.taxiways = r.polylines()?;
            s.parking = r.polylines()?;
        }
        Ok(s)
    }

    /// Runway end markers: position and the direction (degrees) the runway is used in.
    pub fn runway_ends(&self) -> impl Iterator<Item = &Placed> {
        let idx = self.classes.iter().position(|c| c == RUNWAY_CLASS);
        self.objects.iter().filter(move |o| Some(o.class) == idx)
    }
}

#[cfg(test)]
mod tests {
    use super::House;

    #[test]
    fn decodes_house_record() {
        // class 5, dead, yaw -8000 (= -90 degrees); offsets +16000 / -3200 in cell (10, 20).
        let code = 5 | 0x8000 | ((-8000i16 as u16 as i32) << 16);
        let pos = (16000u16 as i32) | ((-3200i16 as u16 as i32) << 16);
        let h = House::decode(10, 20, code, pos);
        assert_eq!(h.class, 5);
        assert!(h.dead);
        assert_eq!(h.yaw, -90.0);
        assert_eq!(h.x, 2100.0);
        assert_eq!(h.y, 3980.0);
    }
}
