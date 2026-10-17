//! DeviceLink (`docs/behavior/devicelink.md`): the UDP interface for external tools described in
//! the install's `DeviceLink.txt`. Queries `R/key\value…` are answered with `A/key\value…`.

use glam::DVec3;
use il2_fm::Sim;
use std::net::{IpAddr, SocketAddr, UdpSocket};

/// Something a client asked the aircraft to do (set keys).
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Power(f64),
    Flaps(f64),
    Aileron(f64),
    Elevator(f64),
    Rudder(f64),
    Brakes(f64),
    PropPitch(f64),
    Trim(usize, f64),
    ToggleEngine,
    /// Next magneto setting.
    Magneto,
    ToggleWep,
    ToggleGear,
    ToggleAirbrake,
    ToggleTailLock,
    /// Trigger group (0..3) and whether to fire.
    Fire(usize, bool),
}

/// What the client can read, gathered by the game each frame.
pub struct Readings<'a> {
    pub sim: &'a Sim,
    pub plane: &'a str,
    pub time_of_day: f64,
    pub cockpit: bool,
    pub cockpits: usize,
    /// Trigger groups being fired.
    pub firing: [bool; 4],
    pub wep: bool,
    pub tail_lock: bool,
}

pub struct DeviceLink {
    socket: UdpSocket,
    allowed: Vec<IpAddr>,
}

/// The runtime's number format: two decimals.
fn fmt(v: f64) -> String {
    format!("{v:.2}")
}

impl DeviceLink {
    /// Opens the port named in the install's `conf.ini` `[DeviceLink]` (`port`, `host`, `IPS`), or
    /// `port_override`. `None` when DeviceLink is off.
    pub fn open(root: &std::path::Path, port_override: Option<u16>) -> Option<DeviceLink> {
        let conf = std::fs::read(root.join("conf.ini"))
            .ok()
            .map(|d| il2_formats::SectFile::parse(&d));
        let sect = conf.as_ref().and_then(|c| c.section_ci("DeviceLink"));
        let get = |k: &str| {
            sect.and_then(|s| s.get_ci(k))
                .map(|v| v.trim_start_matches(['=', ' ']).trim().to_owned())
        };
        let port = port_override.or_else(|| get("port").and_then(|p| p.parse().ok()))?;
        let host = get("host").unwrap_or_else(|| "0.0.0.0".into());
        let allowed = get("IPS")
            .map(|s| {
                s.split_whitespace()
                    .filter_map(|a| a.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        let socket = UdpSocket::bind((host.as_str(), port))
            .map_err(|e| eprintln!("devicelink: port {port}: {e}"))
            .ok()?;
        socket.set_nonblocking(true).ok()?;
        eprintln!("devicelink: listening on {host}:{port}");
        Some(DeviceLink { socket, allowed })
    }

    /// Answers every waiting query; returns the commands the queries carried.
    pub fn poll(&mut self, r: &Readings) -> Vec<Command> {
        let mut commands = Vec::new();
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = self.socket.recv_from(&mut buf) {
            if !self.allowed.is_empty() && !self.allowed.contains(&from.ip()) {
                continue;
            }
            let answers = handle(&buf[..n], r, &mut commands);
            self.reply(&answers, from);
        }
        commands
    }

    fn reply(&self, answers: &[String], to: SocketAddr) {
        let mut packet = String::from("A");
        for a in answers {
            if packet.len() + a.len() > 2047 {
                let _ = self.socket.send_to(packet.as_bytes(), to);
                packet = String::from("A");
            }
            packet += a;
        }
        if packet.len() > 1 {
            let _ = self.socket.send_to(packet.as_bytes(), to);
        }
    }
}

/// Splits a query `R/40/81\1.6e-1` into keys with their values (`\` escapes the next symbol).
pub fn parse(data: &[u8]) -> Option<Vec<(u32, Vec<String>)>> {
    let text: Vec<char> = data.iter().map(|&b| b as char).collect();
    if text.first() != Some(&'R') {
        return None;
    }
    let mut out: Vec<(u32, Vec<String>)> = Vec::new();
    let mut i = 1;
    while i < text.len() {
        if text[i] != '/' {
            return None;
        }
        i += 1;
        let start = i;
        while i < text.len() && text[i].is_ascii_digit() {
            i += 1;
        }
        let key: u32 = text[start..i].iter().collect::<String>().parse().ok()?;
        let mut values = Vec::new();
        while i < text.len() && text[i] == '\\' {
            i += 1;
            let mut v = String::new();
            while i < text.len() && text[i] != '/' {
                if text[i] == '\\' {
                    // An escaped symbol, or the start of the next value.
                    if i + 1 < text.len() && (text[i + 1] == '\\' || text[i + 1] == '/') {
                        v.push(text[i + 1]);
                        i += 2;
                        continue;
                    }
                    break;
                }
                v.push(text[i]);
                i += 1;
            }
            values.push(v);
        }
        out.push((key, values));
    }
    Some(out)
}

fn answer(key: u32, values: &[String]) -> String {
    let mut s = format!("/{key}");
    for v in values {
        s.push('\\');
        for c in v.chars() {
            if c == '\\' || c == '/' {
                s.push('\\');
            }
            s.push(c);
        }
    }
    s
}

/// The get keys OpenIL2 answers.
const GETS: &[u32] = &[
    2, 4, 6, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38, 40, 42, 46, 48, 50, 52, 54, 56, 58, 60, 62,
    64, 66, 68, 70, 72, 74, 80, 82, 84, 86, 88, 90, 92, 94, 96, 98, 104, 106, 108, 164, 172, 174,
    180, 182, 184, 186, 188,
];
/// The set keys OpenIL2 accepts.
const SETS: &[u32] = &[
    81, 83, 85, 87, 89, 91, 93, 95, 97, 99, 103, 105, 107, 109, 165, 173, 175, 181, 183, 185, 187,
    189,
];

fn handle(data: &[u8], r: &Readings, commands: &mut Vec<Command>) -> Vec<String> {
    let Some(query) = parse(data) else {
        return Vec::new();
    };
    let s = r.sim;
    let fwd = s.ori * DVec3::X;
    let mut out = Vec::new();
    for (key, values) in query {
        let num = |i: usize| values.get(i).and_then(|v| v.parse::<f64>().ok());
        let engine = num(0).map(|v| v as usize).filter(|&e| e < s.engines.len());
        let one = |v: f64| vec![fmt(v)];
        let get: Option<Vec<String>> = match key {
            2 => Some(vec!["OpenIL2 DeviceLink 1.0".into()]),
            4 => num(0).map(|k| {
                vec![
                    (k as u32).to_string(),
                    (GETS.contains(&(k as u32)) as u8).to_string(),
                ]
            }),
            6 => num(0).map(|k| {
                vec![
                    (k as u32).to_string(),
                    (SETS.contains(&(k as u32)) as u8).to_string(),
                ]
            }),
            20 => Some(one(r.time_of_day)),
            22 => Some(vec![r.plane.to_owned()]),
            24 => Some(vec![r.cockpits.to_string()]),
            26 => Some(vec![if r.cockpit { "0".into() } else { "-1".into() }]),
            28 => Some(vec![s.engines.len().to_string()]),
            30 => Some(one(s.ias * 3.6)),
            32 => Some(one(s.vel.z)),
            34 => {
                let v = s.ori.inverse() * s.vel;
                Some(one(v.y.atan2(v.x).to_degrees().clamp(-45.0, 45.0)))
            }
            36 => Some(one((s.w.z / 0.25).clamp(-1.0, 1.0))),
            38 => Some(one(-s.w.z.to_degrees())),
            40 => Some(one(s.pos.z)),
            42 => Some(one(
                (90.0 - fwd.y.atan2(fwd.x).to_degrees()).rem_euclid(360.0)
            )),
            46 => Some(one(s.bank_deg())),
            48 => Some(one(fwd.z.clamp(-1.0, 1.0).asin().to_degrees())),
            50 => Some(one(s.fuel)),
            52 => Some(one(s.overload)),
            54 => Some(one(0.0)),
            56 | 58 | 60 => Some(one(s.ct.gear)),
            62 => engine.map(|e| vec![e.to_string(), s.engines[e].magneto.to_string()]),
            106 | 108 => Some(vec![s.engines.first().map_or(0, |e| e.magneto).to_string()]),
            64 => engine.map(|e| vec![e.to_string(), fmt(s.engines[e].rpm())]),
            66 => engine.map(|e| vec![e.to_string(), fmt(s.engines[e].manifold * 1.01325)]),
            68 => engine.map(|e| vec![e.to_string(), fmt(s.engines[e].t_oil_in)]),
            70 => engine.map(|e| vec![e.to_string(), fmt(s.engines[e].t_oil_out)]),
            72 | 74 => engine.map(|e| vec![e.to_string(), fmt(s.engines[e].t_water)]),
            80 => Some(one(s.ct.throttle)),
            82 => Some(one(s.ct.flaps)),
            84 => Some(one(s.ct.aileron)),
            86 => Some(one(s.ct.elevator)),
            88 => Some(one(s.ct.rudder)),
            90 => Some(one(s.ct.brakes)),
            92 => Some(one(s
                .engines
                .first()
                .and_then(|e| e.prop_manual)
                .unwrap_or(1.0))),
            94 => Some(one(s.trim[0])),
            96 => Some(one(s.trim[1])),
            98 => Some(one(s.trim[2])),
            104 => Some(vec![(r.wep as u8).to_string()]),
            164 => Some(one(s.ct.gear)),
            172 => Some(vec![((s.airbrake > 0.5) as u8).to_string()]),
            174 => Some(vec![(r.tail_lock as u8).to_string()]),
            180 | 182 | 184 | 186 => Some(vec![
                (r.firing[((key - 180) / 2) as usize] as u8).to_string()
            ]),
            188 => Some(vec![((r.firing[0] && r.firing[1]) as u8).to_string()]),
            _ => None,
        };
        if let Some(v) = get {
            out.push(answer(key, &v));
            continue;
        }
        let axis = |v: Option<f64>| v.map(|x| x.clamp(-1.0, 1.0));
        let cmd = match key {
            81 => axis(num(0)).map(Command::Power),
            83 => axis(num(0)).map(Command::Flaps),
            85 => axis(num(0)).map(Command::Aileron),
            87 => axis(num(0)).map(Command::Elevator),
            89 => axis(num(0)).map(Command::Rudder),
            91 => axis(num(0)).map(Command::Brakes),
            93 => axis(num(0)).map(Command::PropPitch),
            95 | 97 | 99 => axis(num(0)).map(|v| Command::Trim(((key - 95) / 2) as usize, v)),
            103 => Some(Command::ToggleEngine),
            107 | 109 => Some(Command::Magneto),
            105 => Some(Command::ToggleWep),
            165 => Some(Command::ToggleGear),
            173 => Some(Command::ToggleAirbrake),
            175 => Some(Command::ToggleTailLock),
            181 | 183 | 185 | 187 => Some(Command::Fire(
                ((key - 181) / 2) as usize,
                num(0).unwrap_or(0.0) != 0.0,
            )),
            189 => {
                let on = num(0).unwrap_or(0.0) != 0.0;
                commands.push(Command::Fire(0, on));
                Some(Command::Fire(1, on))
            }
            _ => None,
        };
        commands.extend(cmd);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_syntax() {
        let q = parse(b"R/40/81\\1.6e-1/64\\0").unwrap();
        assert_eq!(
            q,
            vec![
                (40, vec![]),
                (81, vec!["1.6e-1".to_owned()]),
                (64, vec!["0".to_owned()])
            ]
        );
        assert_eq!(answer(40, &["534.30".into()]), "/40\\534.30");
        assert_eq!(answer(4, &["a/b".into()]), "/4\\a\\/b");
        assert!(parse(b"X/1").is_none());
    }

    #[test]
    fn answers_and_commands() {
        let mut ac = il2_fm::Aircraft::empty();
        ac.engines.clear();
        let sim = il2_fm::Sim::new(ac, DVec3::new(0.0, 0.0, 534.3), glam::DQuat::IDENTITY, 50.0);
        let r = Readings {
            sim: &sim,
            plane: "air.BF_109G6",
            time_of_day: 12.5,
            cockpit: true,
            cockpits: 1,
            firing: [false; 4],
            wep: false,
            tail_lock: false,
        };
        let mut cmds = Vec::new();
        let a = handle(b"R/40/22/4\\81/81\\0.5/181\\1", &r, &mut cmds);
        assert_eq!(
            a,
            vec![
                "/40\\534.30".to_owned(),
                "/22\\air.BF_109G6".to_owned(),
                "/4\\81\\0".to_owned()
            ]
        );
        assert_eq!(cmds, vec![Command::Power(0.5), Command::Fire(0, true)]);
    }
}
