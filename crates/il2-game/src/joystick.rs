//! Joystick input through the Linux joystick interface (`/dev/input/js*`), without extra libraries.
//! Events are 8 bytes: `u32` time (ms), `i16` value, `u8` type (1 button, 2 axis; 0x80 marks the initial
//! state), `u8` number.

use std::fs::File;
use std::io::Read;

/// Which joystick axis drives which control; `None` leaves it to the keyboard.
#[derive(Clone, Debug)]
pub struct AxisMap {
    pub aileron: Option<(u8, bool)>,
    pub elevator: Option<(u8, bool)>,
    pub rudder: Option<(u8, bool)>,
    pub throttle: Option<(u8, bool)>,
}

impl Default for AxisMap {
    /// Typical flight stick: X, Y, throttle slider on axis 2 (inverted: forward is full), twist on 3.
    fn default() -> Self {
        AxisMap {
            aileron: Some((0, false)),
            elevator: Some((1, false)),
            throttle: Some((2, true)),
            rudder: Some((3, false)),
        }
    }
}

impl AxisMap {
    /// Parses `aileron=0,elevator=1,rudder=-3,throttle=2` (a minus sign inverts the axis).
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut m = AxisMap {
            aileron: None,
            elevator: None,
            rudder: None,
            throttle: None,
        };
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (k, v) = part
                .split_once('=')
                .ok_or_else(|| format!("bad axis mapping '{part}'"))?;
            let inv = v.starts_with('-');
            let n: u8 = v
                .trim_start_matches('-')
                .parse()
                .map_err(|_| format!("bad axis number in '{part}'"))?;
            let slot = match k {
                "aileron" => &mut m.aileron,
                "elevator" => &mut m.elevator,
                "rudder" => &mut m.rudder,
                "throttle" => &mut m.throttle,
                _ => return Err(format!("unknown control '{k}'")),
            };
            *slot = Some((n, inv));
        }
        Ok(m)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Axis(u8, f64),
    Button(u8, bool),
}

/// Decodes one 8-byte event (native endian); axis values scaled to −1..1.
pub fn decode(b: &[u8; 8]) -> Option<Event> {
    let value = i16::from_ne_bytes([b[4], b[5]]);
    let (kind, number) = (b[6] & !0x80, b[7]);
    match kind {
        1 => Some(Event::Button(number, value != 0)),
        2 => Some(Event::Axis(
            number,
            (value as f64 / 32767.0).clamp(-1.0, 1.0),
        )),
        _ => None,
    }
}

pub struct Joystick {
    file: File,
    buf: Vec<u8>,
    pub axes: [f64; 16],
    pub buttons: [bool; 32],
    /// Axes that have reported at least once.
    seen: [bool; 16],
}

impl Joystick {
    /// Opens the first `/dev/input/js<N>` that can be read, non-blocking.
    pub fn open() -> Option<(Self, String)> {
        use std::os::unix::fs::OpenOptionsExt;
        for n in 0..8 {
            let path = format!("/dev/input/js{n}");
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(0o4000 /* O_NONBLOCK */)
                .open(&path)
            {
                return Some((
                    Joystick {
                        file,
                        buf: Vec::new(),
                        axes: [0.0; 16],
                        buttons: [false; 32],
                        seen: [false; 16],
                    },
                    path,
                ));
            }
        }
        None
    }

    /// Reads all pending events.
    pub fn poll(&mut self) {
        let mut chunk = [0u8; 256];
        while let Ok(n) = self.file.read(&mut chunk) {
            if n == 0 {
                break;
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
        let whole = self.buf.len() / 8 * 8;
        for e in self.buf[..whole].chunks_exact(8) {
            match decode(e.try_into().unwrap()) {
                Some(Event::Axis(i, v)) if (i as usize) < 16 => {
                    self.axes[i as usize] = v;
                    self.seen[i as usize] = true;
                }
                Some(Event::Button(i, v)) if (i as usize) < 32 => self.buttons[i as usize] = v,
                _ => {}
            }
        }
        self.buf.drain(..whole);
    }

    /// The mapped axis value, if that axis exists.
    pub fn axis(&self, a: Option<(u8, bool)>) -> Option<f64> {
        let (i, inv) = a?;
        let i = i as usize;
        (i < 16 && self.seen[i]).then(|| if inv { -self.axes[i] } else { self.axes[i] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_events() {
        let mut b = [0u8; 8];
        b[4..6].copy_from_slice(&(-32767i16).to_ne_bytes());
        b[6] = 2 | 0x80;
        b[7] = 3;
        assert_eq!(decode(&b), Some(Event::Axis(3, -1.0)));
        b[4..6].copy_from_slice(&1i16.to_ne_bytes());
        b[6] = 1;
        assert_eq!(decode(&b), Some(Event::Button(3, true)));
    }

    #[test]
    fn parses_axis_map() {
        let m = AxisMap::parse("aileron=0, elevator=-1,throttle=2").unwrap();
        assert_eq!(m.elevator, Some((1, true)));
        assert_eq!(m.rudder, None);
        assert!(AxisMap::parse("flaps=1").is_err());
    }
}
