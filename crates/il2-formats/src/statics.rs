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

#[derive(Clone, Debug, Default)]
pub struct Statics {
    pub classes: Vec<String>,
    pub objects: Vec<Placed>,
    pub house_classes: Vec<String>,
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
        let s = self.d.get(self.p..self.p + n).ok_or(Error::Bad("truncated"))?;
        self.p += n;
        Ok(s)
    }
    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, Error> {
        let n = self.i32()?;
        usize::try_from(n).ok().filter(|&n| n < 10_000_000).ok_or(Error::Bad("count"))
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
            .map(|_| (0..self.count()?).map(|_| Ok([self.f32()?, self.f32()?])).collect())
            .collect()
    }
}

impl Statics {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let mut r = R { d, p: 0 };
        if r.i32()? != -65535 {
            return Err(Error::Bad("version"));
        }
        for _ in 0..r.count()? {
            r.take(24)?; // bridge: 5 ints and a float
        }
        let mut s = Statics::default();
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
                r.i32()?;
                let n = r.count()?;
                r.take(n * 8)?;
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
