//! Sound: the game's samples and presets (`docs/behavior/sound.md`), mixed in software and played
//! through ALSA. The native sound engine has not been studied; presets are interpreted by name.

mod alsa;

use il2_formats::sectfile::atof;
use il2_formats::SectFile;
use il2_vfs::Vfs;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Mono samples at their own rate.
#[derive(Clone)]
pub struct Clip {
    data: Arc<Vec<f32>>,
    rate: f32,
}

/// Decodes a PCM WAV file (8 or 16 bit, mono or stereo) to mono.
pub fn decode_wav(d: &[u8]) -> Option<Clip> {
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        return None;
    }
    let (mut p, mut fmt, mut data) = (12, None, None);
    while p + 8 <= d.len() {
        let id = &d[p..p + 4];
        let len = u32::from_le_bytes(d[p + 4..p + 8].try_into().ok()?) as usize;
        let body = d.get(p + 8..(p + 8 + len).min(d.len()))?;
        match id {
            b"fmt " if body.len() >= 16 => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        p += 8 + len + (len & 1);
    }
    let (fmt, data) = (fmt?, data?);
    let u16_at = |i: usize| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
    let (tag, channels, rate, bits) = (
        u16_at(0),
        u16_at(2).max(1) as usize,
        u32::from_le_bytes(fmt[4..8].try_into().ok()?),
        u16_at(14),
    );
    if tag != 1 {
        return None;
    }
    let samples: Vec<f32> = match bits {
        16 => data
            .chunks_exact(2 * channels)
            .map(|f| {
                f.chunks_exact(2)
                    .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
                    .sum::<f32>()
                    / channels as f32
            })
            .collect(),
        8 => data
            .chunks_exact(channels)
            .map(|f| f.iter().map(|&s| (s as f32 - 128.0) / 128.0).sum::<f32>() / channels as f32)
            .collect(),
        _ => return None,
    };
    Some(Clip {
        data: Arc::new(samples),
        rate: rate as f32,
    })
}

struct Voice {
    key: u64,
    clip: Clip,
    pos: f64,
    pitch: f32,
    gain: f32,
    looping: bool,
}

#[derive(Default)]
struct Mixer {
    voices: Vec<Voice>,
    rate: f32,
}

impl Mixer {
    fn render(&mut self, out: &mut [i16]) {
        let frames = out.len() / 2;
        let mut acc = vec![0f32; frames];
        for v in &mut self.voices {
            let step = (v.clip.rate / self.rate * v.pitch) as f64;
            let n = v.clip.data.len();
            if n == 0 {
                continue;
            }
            for a in acc.iter_mut() {
                if v.pos >= n as f64 {
                    if v.looping {
                        v.pos %= n as f64;
                    } else {
                        break;
                    }
                }
                let i = v.pos as usize;
                let f = (v.pos - i as f64) as f32;
                let s0 = v.clip.data[i];
                let s1 = v.clip.data[if i + 1 < n {
                    i + 1
                } else if v.looping {
                    0
                } else {
                    i
                }];
                *a += (s0 + (s1 - s0) * f) * v.gain;
                v.pos += step;
            }
        }
        self.voices
            .retain(|v| v.looping || v.pos < v.clip.data.len() as f64);
        for (i, a) in acc.iter().enumerate() {
            // Soft limiter.
            let s = (a.tanh() * 32000.0) as i16;
            out[2 * i] = s;
            out[2 * i + 1] = s;
        }
    }
}

/// A sample's control curve over a value (`minlo..minhi` fade in, `maxlo..maxhi` fade out).
#[derive(Clone, Copy, Debug)]
struct Bounds {
    minlo: f32,
    minhi: f32,
    maxlo: f32,
    maxhi: f32,
    value: f32,
    pmin: f32,
    pmax: f32,
}

impl Bounds {
    fn gain(&self, x: f32) -> f32 {
        let up = if self.minhi > self.minlo {
            ((x - self.minlo) / (self.minhi - self.minlo)).clamp(0.0, 1.0)
        } else if x >= self.minlo {
            1.0
        } else {
            0.0
        };
        let down = if self.maxhi > self.maxlo {
            1.0 - ((x - self.maxlo) / (self.maxhi - self.maxlo)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        up * down
    }

    fn pitch(&self, x: f32) -> f32 {
        let span = (self.maxhi - self.minlo).max(1e-3);
        let t = ((x - self.minlo) / span).clamp(0.0, 1.0);
        let p = self.pmin + (self.pmax - self.pmin) * t;
        if p > 0.0 {
            p
        } else {
            1.0
        }
    }
}

#[derive(Clone, Debug)]
struct PresetSample {
    file: String,
    looping: bool,
    /// Listening environments the sample belongs to (`env`); empty: all.
    env: Vec<i32>,
    controls: Vec<(String, Bounds)>,
}

#[derive(Clone, Debug, Default)]
pub struct Preset {
    samples: Vec<PresetSample>,
    /// Sound pressure level (`[spl]`), dB.
    spl: f32,
}

impl Preset {
    fn parse(d: &[u8]) -> Self {
        let f = SectFile::parse(d);
        let spl = f
            .section_ci("spl")
            .and_then(|s| s.entries.first())
            .and_then(|(k, _)| atof(k))
            .unwrap_or(100.0);
        let names: Vec<String> = f
            .section_ci("samples")
            .map(|s| s.entries.iter().map(|(k, _)| k.clone()).collect())
            .unwrap_or_default();
        let samples = names
            .into_iter()
            .map(|file| {
                let sect = f.section_ci(&format!("sample.{file}"));
                let get = |k: &str| sect.and_then(|s| s.get_ci(k)).map(str::trim);
                let controls = get("controls")
                    .map(|c| {
                        c.split_whitespace()
                            .filter_map(|name| {
                                let b = f.section_ci(&format!("sample.{file}.{name}"))?;
                                let n = |k: &str| b.get_ci(k).and_then(atof).unwrap_or(0.0);
                                Some((
                                    name.to_owned(),
                                    Bounds {
                                        minlo: n("minlo"),
                                        minhi: n("minhi"),
                                        maxlo: n("maxlo"),
                                        maxhi: n("maxhi"),
                                        value: n("value"),
                                        pmin: n("pmin"),
                                        pmax: n("pmax"),
                                    },
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                PresetSample {
                    looping: get("infinite").and_then(atof).unwrap_or(0.0) != 0.0,
                    env: get("env")
                        .map(|e| {
                            e.split_whitespace()
                                .filter_map(|v| v.parse().ok())
                                .collect()
                        })
                        .unwrap_or_default(),
                    controls,
                    file,
                }
            })
            .collect();
        Preset { samples, spl }
    }
}

/// The listener position class: 0 inside the cockpit, 2 outside nearby (the chase view).
pub const ENV_OUTSIDE: i32 = 2;

pub struct Audio {
    mixer: Option<Arc<Mutex<Mixer>>>,
    clips: HashMap<String, Option<Clip>>,
    presets: HashMap<String, Option<Arc<Preset>>>,
    rng: u64,
}

impl Audio {
    /// Starts the output thread; without a sound device the game runs silently.
    pub fn start() -> Self {
        let mixer = match alsa::Pcm::open(44_100, 60) {
            Ok(mut pcm) => {
                let m = Arc::new(Mutex::new(Mixer {
                    voices: Vec::new(),
                    rate: pcm.rate as f32,
                }));
                let mm = m.clone();
                std::thread::spawn(move || {
                    let mut buf = vec![0i16; 2 * 512];
                    loop {
                        mm.lock().unwrap().render(&mut buf);
                        pcm.write(&buf);
                    }
                });
                Some(m)
            }
            Err(e) => {
                eprintln!("sound: {e}; running silent");
                None
            }
        };
        Audio {
            mixer,
            clips: HashMap::new(),
            presets: HashMap::new(),
            rng: 0x50D,
        }
    }

    fn clip(&mut self, vfs: &Vfs, file: &str) -> Option<Clip> {
        self.clips
            .entry(file.to_ascii_lowercase())
            .or_insert_with(|| {
                vfs.read(&format!("samples/{file}"))
                    .ok()
                    .and_then(|d| decode_wav(&d))
            })
            .clone()
    }

    pub fn preset(&mut self, vfs: &Vfs, name: &str) -> Option<Arc<Preset>> {
        self.presets
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| {
                vfs.read(&format!("presets/sounds/{name}.prs"))
                    .ok()
                    .map(|d| Arc::new(Preset::parse(&d)))
            })
            .clone()
    }

    /// Updates (or starts) the looping voices of a preset under key `key`, from the control values.
    /// `level` scales the loudness (distance, volume).
    pub fn drive(
        &mut self,
        vfs: &Vfs,
        key: u64,
        preset: &Preset,
        controls: &[(&str, f32)],
        env: i32,
        level: f32,
    ) {
        let Some(m) = self.mixer.clone() else { return };
        let mut wanted = Vec::new();
        for (i, s) in preset.samples.iter().enumerate() {
            if !s.looping || (!s.env.is_empty() && !s.env.contains(&env)) {
                continue;
            }
            let mut gain = 1.0;
            let mut pitch = 1.0;
            for (name, b) in &s.controls {
                let x = controls
                    .iter()
                    .find(|(n, _)| n == name)
                    .map_or(0.0, |(_, v)| *v);
                gain *= b.gain(x) * if b.value > 0.0 { b.value / 10.0 } else { 1.0 };
                pitch *= b.pitch(x);
            }
            let Some(clip) = self.clip(vfs, &s.file) else {
                continue;
            };
            wanted.push((key * 64 + i as u64, clip, gain * level, pitch));
        }
        let mut mx = m.lock().unwrap();
        for (k, clip, gain, pitch) in wanted {
            match mx.voices.iter_mut().find(|v| v.key == k) {
                Some(v) => {
                    v.gain = gain;
                    v.pitch = pitch;
                }
                None if gain > 0.0 => mx.voices.push(Voice {
                    key: k,
                    clip,
                    pos: 0.0,
                    pitch,
                    gain,
                    looping: true,
                }),
                None => {}
            }
        }
    }

    /// Stops the looping voices started under `key`.
    pub fn stop(&mut self, key: u64) {
        if let Some(m) = &self.mixer {
            m.lock()
                .unwrap()
                .voices
                .retain(|v| !v.looping || v.key / 64 != key);
        }
    }

    /// Plays one random sample of a preset once.
    pub fn play(&mut self, vfs: &Vfs, preset: &Preset, level: f32) {
        let Some(m) = self.mixer.clone() else { return };
        if preset.samples.is_empty() {
            return;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let s = &preset.samples[(self.rng % preset.samples.len() as u64) as usize];
        let Some(clip) = self.clip(vfs, &s.file) else {
            return;
        };
        let gain = level * 10f32.powf((preset.spl - 130.0) / 40.0);
        m.lock().unwrap().voices.push(Voice {
            key: u64::MAX,
            clip,
            pos: 0.0,
            pitch: 1.0,
            gain,
            looping: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_curve() {
        let b = Bounds {
            minlo: 400.0,
            minhi: 1000.0,
            maxlo: 1500.0,
            maxhi: 1900.0,
            value: 5.0,
            pmin: 0.5,
            pmax: 1.5,
        };
        assert_eq!(b.gain(300.0), 0.0);
        assert_eq!(b.gain(1200.0), 1.0);
        assert!((b.gain(1700.0) - 0.5).abs() < 1e-6);
        assert_eq!(b.gain(2000.0), 0.0);
        assert!((b.pitch(400.0) - 0.5).abs() < 1e-6 && (b.pitch(1900.0) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn decodes_pcm16() {
        let mut d = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        d.extend(16u32.to_le_bytes());
        d.extend([1, 0, 1, 0]);
        d.extend(44100u32.to_le_bytes());
        d.extend(88200u32.to_le_bytes());
        d.extend([2, 0, 16, 0]);
        d.extend(b"data");
        d.extend(4u32.to_le_bytes());
        d.extend(16384i16.to_le_bytes());
        d.extend((-16384i16).to_le_bytes());
        let c = decode_wav(&d).unwrap();
        assert_eq!(c.data.as_slice(), &[0.5, -0.5]);
    }
}
