//! Multiplayer (`docs/behavior/multiplayer.md`), OpenIL2's own protocol over UDP: a host relays
//! the pilots' states, rounds and damage; every machine flies its own aircraft.

use crate::airframe::Airframe;
use crate::track::{self, AircraftState, BulletSpawn, R, W};
use crate::world::{self, AircraftAssets};
use glam::{DMat4, DVec3};
use il2_fm::Sim;
use il2_render::{GpuMesh, MeshData, Renderer};
use il2_vfs::Vfs;
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// First hit-target id of remote aircraft (`hitbox::Target::id`).
pub const ID_BASE: usize = 3_000_000;

const HELLO: u8 = 1;
const WELCOME: u8 = 2;
const PEER: u8 = 3;
const STATE: u8 = 4;
const DAMAGE: u8 = 5;
const FIRE: u8 = 6;
const BYE: u8 = 7;
const CHAT: u8 = 8;

/// Seconds between state packets, and how far behind real time remote aircraft are shown.
const SEND_STEP: f64 = 0.05;
const DELAY: f64 = 0.1;

pub struct Remote {
    pub id: u32,
    pub name: String,
    pub army: u8,
    pub assets: AircraftAssets,
    mesh: Option<MeshData>,
    gpu: Option<GpuMesh>,
    states: VecDeque<(f64, AircraftState)>,
    /// Pose holder for drawing, hit tests and damage.
    pub sim: Sim,
    pub airframe: Airframe,
}

enum Role {
    Host {
        clients: Vec<(u32, SocketAddr)>,
        options: Vec<String>,
        next_id: u32,
    },
    Client {
        server: SocketAddr,
    },
}

/// What arrived for the game to act on.
pub enum Event {
    /// Damage to our own aircraft.
    Damage(String, usize),
    /// Rounds another pilot fired (for show).
    Fire(Vec<BulletSpawn>),
    Message(String),
}

pub struct Net {
    socket: UdpSocket,
    role: Role,
    pub id: u32,
    pub name: String,
    pub remotes: Vec<Remote>,
    start: Instant,
    next_send: f64,
    last_bullet: u64,
    pending_fire: Vec<BulletSpawn>,
    peers_info: HashMap<u32, (String, String, u8)>,
}

fn packet(kind: u8, f: impl FnOnce(&mut W)) -> Vec<u8> {
    let mut buf = vec![kind];
    f(&mut W(&mut buf));
    buf
}

impl Net {
    /// Hosts a game on `port`; `options` set the flight up on the clients (map or mission, time).
    pub fn host(port: u16, name: &str, options: Vec<String>) -> std::io::Result<Net> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_nonblocking(true)?;
        eprintln!("net: hosting on port {port}");
        Ok(Net {
            socket,
            role: Role::Host {
                clients: Vec::new(),
                options,
                next_id: 2,
            },
            id: 1,
            name: name.to_owned(),
            remotes: Vec::new(),
            start: Instant::now(),
            next_send: 0.0,
            last_bullet: 0,
            pending_fire: Vec::new(),
            peers_info: HashMap::new(),
        })
    }

    /// Joins a host; returns the connection and the options the host's flight uses.
    pub fn join(
        server: &str,
        name: &str,
        class: &str,
        army: u8,
    ) -> Result<(Net, Vec<String>), String> {
        let server: SocketAddr = std::net::ToSocketAddrs::to_socket_addrs(server)
            .map_err(|e| e.to_string())?
            .next()
            .ok_or("no address")?;
        let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| e.to_string())?;
        let hello = packet(HELLO, |w| {
            w.str(name);
            w.str(class);
            w.u8(army);
        });
        let mut buf = [0u8; 4096];
        for _ in 0..20 {
            socket.send_to(&hello, server).map_err(|e| e.to_string())?;
            if let Ok((n, from)) = socket.recv_from(&mut buf) {
                if from == server && n > 0 && buf[0] == WELCOME {
                    let mut r = R { d: &buf[..n], p: 1 };
                    let id = r.u32().ok_or("bad welcome")?;
                    let k = r.u16().ok_or("bad welcome")?;
                    let options = (0..k)
                        .map(|_| r.str())
                        .collect::<Option<Vec<_>>>()
                        .ok_or("bad welcome")?;
                    socket.set_nonblocking(true).map_err(|e| e.to_string())?;
                    eprintln!("net: joined {server} as pilot {id}");
                    let net = Net {
                        socket,
                        role: Role::Client { server },
                        id,
                        name: name.to_owned(),
                        remotes: Vec::new(),
                        start: Instant::now(),
                        next_send: 0.0,
                        last_bullet: 0,
                        pending_fire: Vec::new(),
                        peers_info: HashMap::new(),
                    };
                    return Ok((net, options));
                }
            }
        }
        Err(format!("no answer from {server}"))
    }

    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Sends to the host, or (as host) to every client except `except`.
    fn send(&self, data: &[u8], except: Option<SocketAddr>) {
        match &self.role {
            Role::Client { server } => {
                let _ = self.socket.send_to(data, server);
            }
            Role::Host { clients, .. } => {
                for (_, a) in clients {
                    if Some(*a) != except {
                        let _ = self.socket.send_to(data, a);
                    }
                }
            }
        }
    }

    /// Receives everything waiting; loads the models of new pilots.
    pub fn poll(
        &mut self,
        vfs: &Vfs,
        fms: &il2_data::FlightModels,
        field: &il2_formats::map::HeightField,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        let mut buf = [0u8; 65536];
        let now = self.now();
        while let Ok((n, from)) = self.socket.recv_from(&mut buf) {
            let data = buf[..n].to_vec();
            if data.is_empty() {
                continue;
            }
            let mut r = R { d: &data, p: 1 };
            match data[0] {
                HELLO => {
                    let (Some(name), Some(class), Some(army)) = (r.str(), r.str(), r.u8()) else {
                        continue;
                    };
                    let Role::Host {
                        clients,
                        options,
                        next_id,
                    } = &mut self.role
                    else {
                        continue;
                    };
                    let id = match clients.iter().find(|c| c.1 == from) {
                        Some(c) => c.0,
                        None => {
                            let id = *next_id;
                            *next_id += 1;
                            clients.push((id, from));
                            id
                        }
                    };
                    // New pilots start spread around the host's start, 1.5 km apart.
                    let mut opts = options.clone();
                    let a = id as f64 * 2.4;
                    opts.extend([
                        "--spawn-offset".into(),
                        format!("{:.0},{:.0}", a.cos() * 1500.0, a.sin() * 1500.0),
                    ]);
                    let welcome = packet(WELCOME, |w| {
                        w.u32(id);
                        w.u16(opts.len());
                        for o in &opts {
                            w.str(o);
                        }
                    });
                    let _ = self.socket.send_to(&welcome, from);
                    events.push(Event::Message(format!("{name} joined")));
                    self.peers_info
                        .insert(id, (name.clone(), class.clone(), army));
                    // Everyone learns about everyone (the host included).
                    let announce = |id: u32, name: &str, class: &str, army: u8| {
                        packet(PEER, |w| {
                            w.u32(id);
                            w.str(name);
                            w.str(class);
                            w.u8(army);
                        })
                    };
                    let all: Vec<(u32, String, String, u8)> = self
                        .peers_info
                        .iter()
                        .map(|(i, (n, c, a))| (*i, n.clone(), c.clone(), *a))
                        .collect();
                    for (i, n, c, a) in &all {
                        self.send(&announce(*i, n, c, *a), None);
                    }
                    self.add_remote(vfs, fms, field, id, &name, &class, army);
                }
                PEER => {
                    let (Some(id), Some(name), Some(class), Some(army)) =
                        (r.u32(), r.str(), r.str(), r.u8())
                    else {
                        continue;
                    };
                    if id != self.id && !self.remotes.iter().any(|x| x.id == id) {
                        events.push(Event::Message(format!("{name} is here")));
                        self.add_remote(vfs, fms, field, id, &name, &class, army);
                    }
                }
                STATE => {
                    let Some(s) = track::get_state(&mut r) else {
                        continue;
                    };
                    if let Some(rm) = self.remotes.iter_mut().find(|x| x.id == s.id) {
                        rm.states.push_back((now, s));
                        while rm.states.len() > 40 {
                            rm.states.pop_front();
                        }
                    }
                    if matches!(self.role, Role::Host { .. }) {
                        self.send(&data, Some(from));
                    }
                }
                DAMAGE => {
                    let (Some(target), Some(chunk), Some(levels)) = (r.u32(), r.str(), r.u8())
                    else {
                        continue;
                    };
                    if target == self.id {
                        events.push(Event::Damage(chunk, levels as usize));
                    } else if let Some(rm) = self.remotes.iter_mut().find(|x| x.id == target) {
                        rm.airframe
                            .next_dmg_levels(&mut rm.sim, levels as usize, &chunk);
                        rm.airframe.log.clear();
                    }
                    if matches!(self.role, Role::Host { .. }) {
                        self.send(&data, Some(from));
                    }
                }
                FIRE => {
                    let _shooter = r.u32();
                    let k = r.u16().unwrap_or(0);
                    let spawns: Vec<BulletSpawn> =
                        (0..k).filter_map(|_| track::get_bullet(&mut r)).collect();
                    events.push(Event::Fire(spawns));
                    if matches!(self.role, Role::Host { .. }) {
                        self.send(&data, Some(from));
                    }
                }
                CHAT => {
                    let (Some(_id), Some(text)) = (r.u32(), r.str()) else {
                        continue;
                    };
                    events.push(Event::Message(text));
                    if matches!(self.role, Role::Host { .. }) {
                        self.send(&data, Some(from));
                    }
                }
                BYE => {
                    let Some(id) = r.u32() else { continue };
                    if let Some(i) = self.remotes.iter().position(|x| x.id == id) {
                        events.push(Event::Message(format!("{} left", self.remotes[i].name)));
                        self.remotes.remove(i);
                    }
                    if let Role::Host { clients, .. } = &mut self.role {
                        clients.retain(|c| c.0 != id);
                        self.send(&data, Some(from));
                    }
                }
                _ => {}
            }
        }
        // Remote aircraft shown a little behind real time, between the states around then.
        let show = now - DELAY;
        for rm in &mut self.remotes {
            let st = &rm.states;
            let Some(i) = st.iter().position(|(t, _)| *t > show) else {
                if let Some((_, s)) = st.back() {
                    apply(&mut rm.sim, s, s, 1.0);
                }
                continue;
            };
            if i == 0 {
                apply(&mut rm.sim, &st[0].1, &st[0].1, 1.0);
            } else {
                let (t0, a) = &st[i - 1];
                let (t1, b) = &st[i];
                let k = ((show - t0) / (t1 - t0).max(1e-3)).clamp(0.0, 1.0) as f32;
                apply(&mut rm.sim, a, b, k);
            }
            rm.airframe.update(&mut rm.sim, field, 0.0);
        }
        events
    }

    /// The host announces its own aircraft to the pilots who join.
    pub fn set_own(&mut self, class: &str, army: u8) {
        if matches!(self.role, Role::Host { .. }) {
            self.peers_info
                .insert(self.id, (self.name.clone(), class.to_owned(), army));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_remote(
        &mut self,
        vfs: &Vfs,
        fms: &il2_data::FlightModels,
        field: &il2_formats::map::HeightField,
        id: u32,
        name: &str,
        class: &str,
        army: u8,
    ) {
        if id == self.id || self.remotes.iter().any(|r| r.id == id) {
            return;
        }
        let Ok(mut assets) = world::load_aircraft_with(vfs, fms, class, false) else {
            eprintln!("net: no aircraft {class} for {name}");
            return;
        };
        let mesh = std::mem::take(&mut assets.mesh);
        let sim = world::spawn_airborne(assets.fm.clone(), field, 1000.0, 0.0);
        let airframe = Airframe::new(
            assets.rig.names.clone(),
            assets.rig.parents.clone(),
            &assets.hidden,
            assets.gear_chunks.clone(),
        );
        self.remotes.push(Remote {
            id,
            name: name.to_owned(),
            army,
            assets,
            mesh: Some(mesh),
            gpu: None,
            states: VecDeque::new(),
            sim,
            airframe,
        });
    }

    /// Sends our state, the rounds we fired and the damage we dealt.
    pub fn send_own(&mut self, sim: &Sim, bullets: &[crate::weapons::Bullet]) {
        let now = self.now();
        let last = self.last_bullet;
        for b in bullets.iter().filter(|b| b.id > last && b.owner == 0) {
            self.last_bullet = self.last_bullet.max(b.id);
            self.pending_fire.push(BulletSpawn {
                pos: b.pos,
                vel: b.vel.as_vec3(),
                gun: b.spec.name.clone(),
                kind: b.kind as u8,
                tracer: b.tracer,
                life: b.life as f32,
                fuse: b.fuse.map(|v| v as f32),
            });
        }
        for i in 0..self.remotes.len() {
            let target = self.remotes[i].id;
            for (chunk, levels) in std::mem::take(&mut self.remotes[i].airframe.log) {
                let p = packet(DAMAGE, |w| {
                    w.u32(target);
                    w.str(&chunk);
                    w.u8(levels.min(255) as u8);
                });
                self.send(&p, None);
            }
        }
        if now < self.next_send {
            return;
        }
        self.next_send = now + SEND_STEP;
        let s = track::AircraftState {
            id: self.id,
            pos: sim.pos,
            ori: sim.ori.as_quat(),
            vel: sim.vel.as_vec3(),
            ct: [
                sim.ct.aileron,
                sim.ct.elevator,
                sim.ct.rudder,
                sim.ct.flaps,
                sim.ct.gear,
                sim.ct.throttle,
                sim.airbrake,
            ]
            .map(|v| v as f32),
            destroyed: sim.structure.destroyed,
        };
        let p = packet(STATE, |w| track::put_state(w, &s));
        self.send(&p, None);
        if !self.pending_fire.is_empty() {
            let fire = std::mem::take(&mut self.pending_fire);
            for chunk in fire.chunks(400) {
                let p = packet(FIRE, |w| {
                    w.u32(self.id);
                    w.u16(chunk.len());
                    for b in chunk {
                        track::put_bullet(w, b);
                    }
                });
                self.send(&p, None);
            }
        }
    }

    pub fn chat(&self, text: &str) {
        let p = packet(CHAT, |w| {
            w.u32(self.id);
            w.str(text);
        });
        self.send(&p, None);
    }

    pub fn bye(&self) {
        let p = packet(BYE, |w| w.u32(self.id));
        self.send(&p, None);
    }

    /// Hit targets for the remote aircraft.
    pub fn targets(&self, out: &mut Vec<crate::hitbox::Target>) {
        for r in &self.remotes {
            if r.states.is_empty() {
                continue;
            }
            let mut nodes = world::pose(&r.assets, &r.sim.ct);
            r.airframe.apply(&mut nodes, &r.sim);
            out.push(crate::hitbox::Target::new(
                ID_BASE + r.id as usize,
                r.sim.pos,
                r.sim.ori,
                &nodes,
                &r.assets.boxes,
            ));
        }
    }

    /// Our round hit a remote aircraft: the damage is worked out here and sent to its owner.
    pub fn shot(&mut self, id: usize, node: usize, tnt: f64) {
        let Some(r) = self
            .remotes
            .iter_mut()
            .find(|r| ID_BASE + r.id as usize == id)
        else {
            return;
        };
        let toughness = r.assets.toughness;
        r.airframe.shot(&mut r.sim, node, tnt, &toughness);
    }

    /// Places the remote aircraft relative to the camera and returns their meshes.
    pub fn prepare(&mut self, rend: &Renderer, eye: DVec3) -> Vec<&GpuMesh> {
        for r in &mut self.remotes {
            if r.states.is_empty() {
                continue;
            }
            if r.gpu.is_none() {
                if let Some(m) = r.mesh.take() {
                    r.gpu = Some(rend.upload(&m));
                }
            }
            let Some(g) = &r.gpu else { continue };
            rend.set_transform(
                g,
                DMat4::from_rotation_translation(r.sim.ori, r.sim.pos - eye).as_mat4(),
            );
            let mut nodes = world::pose(&r.assets, &r.sim.ct);
            r.airframe.apply(&mut nodes, &r.sim);
            rend.set_nodes(g, &nodes);
        }
        self.remotes
            .iter()
            .filter(|r| !r.states.is_empty())
            .filter_map(|r| r.gpu.as_ref())
            .collect()
    }
}

fn apply(sim: &mut Sim, a: &AircraftState, b: &AircraftState, k: f32) {
    let (pos, ori) = track::lerp(a, b, k);
    sim.pos = pos;
    sim.ori = ori;
    sim.vel = b.vel.as_dvec3();
    let c = b.ct;
    sim.ct.aileron = c[0] as f64;
    sim.ct.elevator = c[1] as f64;
    sim.ct.rudder = c[2] as f64;
    sim.ct.flaps = c[3] as f64;
    sim.ct.gear = c[4] as f64;
    sim.ct.throttle = c[5] as f64;
    sim.airbrake = c[6] as f64;
    sim.structure.destroyed = b.destroyed;
}
