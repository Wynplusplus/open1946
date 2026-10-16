//! Menus (`docs/behavior/menus.md`): main menu, single missions with their briefings, and the
//! Quick Mission Builder. Labels come from the install's `i18n/gui.properties`; a flight runs as a
//! child process of the menu (`openil2 --mission …`).

use crate::hud::Hud;
use il2_formats::SectFile;
use il2_render::Renderer;
use il2_vfs::Vfs;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// The user's language: `IL2_LANG`, else `LANG` (`de_DE.UTF-8` → `de`); English is the default.
pub fn language() -> Option<String> {
    let l = std::env::var("IL2_LANG")
        .ok()
        .or_else(|| std::env::var("LANG").ok())?;
    let l = l.split(['_', '.']).next()?.to_ascii_lowercase();
    (["de", "fr", "pl", "cs", "hu", "fi", "it", "ru", "ja"].contains(&l.as_str())).then_some(l)
}

/// A localized `.properties` file: `base_<lang>.properties` if present, else `base.properties`.
fn properties_vfs(vfs: &Vfs, base: &str, lang: &Option<String>) -> HashMap<String, String> {
    let localized = lang
        .as_ref()
        .and_then(|l| vfs.read(&format!("{base}_{l}.properties")).ok());
    localized
        .or_else(|| vfs.read(&format!("{base}.properties")).ok())
        .map(|d| il2_formats::properties::parse(&d))
        .unwrap_or_default()
}

fn properties_file(path_no_ext: &Path, lang: &Option<String>) -> HashMap<String, String> {
    let try_read = |p: PathBuf| std::fs::read(p).ok();
    let localized = lang.as_ref().and_then(|l| {
        let mut p = path_no_ext.as_os_str().to_owned();
        p.push(format!("_{l}.properties"));
        try_read(PathBuf::from(p))
    });
    localized
        .or_else(|| {
            let mut p = path_no_ext.as_os_str().to_owned();
            p.push(".properties");
            try_read(PathBuf::from(p))
        })
        .map(|d| il2_formats::properties::parse(&d))
        .unwrap_or_default()
}

struct MissionEntry {
    /// Path relative to the install, as `--mission` takes it.
    path: String,
    name: String,
    briefing: String,
}

struct Group {
    name: String,
    items: Vec<Group>,
    missions: Vec<MissionEntry>,
    /// Campaigns: the stages, each with its alternative missions.
    stages: Vec<Vec<String>>,
}

fn sorted_dirs(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(p)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// `Missions/Single/<country>/<aircraft>/*.mis`.
fn single_missions(root: &Path, lang: &Option<String>) -> Vec<Group> {
    let base = root.join("Missions/Single");
    let mut out = Vec::new();
    for c in sorted_dirs(&base) {
        let mut planes = Vec::new();
        for pdir in sorted_dirs(&c) {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&pdir)
                .map(|r| {
                    r.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mis")))
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            let missions: Vec<MissionEntry> = files
                .iter()
                .map(|f| {
                    let props = properties_file(&f.with_extension(""), lang);
                    let stem = f
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    MissionEntry {
                        path: f
                            .strip_prefix(root)
                            .unwrap_or(f)
                            .to_string_lossy()
                            .into_owned(),
                        name: props.get("Name").cloned().unwrap_or(stem),
                        briefing: props
                            .get("Description")
                            .or_else(|| props.get("Short"))
                            .cloned()
                            .unwrap_or_default(),
                    }
                })
                .collect();
            if !missions.is_empty() {
                planes.push(Group {
                    name: name_of(&pdir),
                    items: Vec::new(),
                    missions,
                    stages: Vec::new(),
                });
            }
        }
        if !planes.is_empty() {
            out.push(Group {
                name: name_of(&c),
                items: planes,
                missions: Vec::new(),
                stages: Vec::new(),
            });
        }
    }
    out
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One aircraft for the QMB lists: class (`air.BF_109G6`), display name, side.
#[derive(Clone)]
struct Plane {
    class: String,
    name: String,
    army: u8,
}

fn planes(vfs: &Vfs, lang: &Option<String>) -> Vec<Plane> {
    let names = properties_vfs(vfs, "i18n/plane", lang);
    let air = vfs
        .read("com/maddox/il2/objects/air.ini")
        .map(|d| SectFile::parse(&d))
        .unwrap_or_default();
    let mut out: Vec<Plane> = Vec::new();
    for (key, v) in air
        .section_ci("AIR")
        .map(|s| s.entries.as_slice())
        .unwrap_or_default()
    {
        let mut t = v.split_whitespace();
        let Some(class) = t.next() else { continue };
        let army = t.next().and_then(|a| a.parse().ok()).unwrap_or(1);
        if out.iter().any(|p| p.class == class) {
            continue;
        }
        out.push(Plane {
            class: class.to_owned(),
            name: names.get(key).cloned().unwrap_or_else(|| key.clone()),
            army,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A QMB template: `Missions/Quick/<Map>/<Map><Red|Blue><Situation><nn>.mis`.
struct Template {
    path: PathBuf,
    map: String,
    red: bool,
    situation: String,
}

fn templates(root: &Path) -> Vec<Template> {
    let mut out = Vec::new();
    for d in sorted_dirs(&root.join("Missions/Quick")) {
        let map = name_of(&d);
        let mut files: Vec<PathBuf> = std::fs::read_dir(&d)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        for f in files {
            let stem = f
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Some(rest) = stem.strip_prefix(&map) else {
                continue;
            };
            let (red, rest) = if let Some(r) = rest.strip_prefix("Red") {
                (true, r)
            } else if let Some(r) = rest.strip_prefix("Blue") {
                (false, r)
            } else {
                continue;
            };
            out.push(Template {
                path: f.clone(),
                map: map.clone(),
                red,
                situation: rest.to_owned(),
            });
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Main,
    Single,
    Quick,
    Career,
    Tracks,
    Dynamic,
}

/// The dynamic campaign's choices and progress.
#[derive(Default)]
struct Dynamic {
    theaters: Vec<String>,
    sel: usize,
    loaded: Option<crate::dgen::Theater>,
    red: bool,
    plane: usize,
    sorties: u32,
    won: u32,
    last: String,
}

fn dgen_path() -> PathBuf {
    data_dir().join("dynamic.ini")
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("openil2")
}

/// Recorded flights, newest first.
fn tracks() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(data_dir().join("tracks"))
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "otrk"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

fn campaigns(root: &Path, lang: &Option<String>) -> Vec<Group> {
    let base = root.join("Missions/Campaign");
    let mut out = Vec::new();
    for c in sorted_dirs(&base) {
        let mut items = Vec::new();
        for d in sorted_dirs(&c) {
            let Ok(ini) = std::fs::read(d.join("campaign.ini")) else {
                continue;
            };
            let text: String = ini.iter().map(|&b| b as char).collect();
            let mut in_list = false;
            let mut stages = Vec::new();
            for line in text.lines() {
                let l = line.trim();
                if l.starts_with('[') {
                    in_list = l.eq_ignore_ascii_case("[list]");
                    continue;
                }
                if in_list && !l.is_empty() {
                    stages.push(l.split_whitespace().map(str::to_owned).collect::<Vec<_>>());
                }
            }
            if stages.is_empty() {
                continue;
            }
            let info = properties_file(&d.join("info"), lang);
            let rel = d
                .strip_prefix(root)
                .unwrap_or(&d)
                .to_string_lossy()
                .into_owned();
            items.push(Group {
                name: info.get("Name").cloned().unwrap_or_else(|| name_of(&d)),
                items: Vec::new(),
                missions: vec![MissionEntry {
                    path: rel,
                    name: String::new(),
                    briefing: info.get("Description").cloned().unwrap_or_default(),
                }],
                stages,
            });
        }
        if !items.is_empty() {
            out.push(Group {
                name: name_of(&c),
                items,
                missions: Vec::new(),
                stages: Vec::new(),
            });
        }
    }
    out
}

/// The pilot's progress: campaign directory (install-relative) and the next stage.
#[derive(Default)]
struct Career {
    campaign: String,
    stage: usize,
    flown: u32,
    won: u32,
}

fn career_path() -> PathBuf {
    data_dir().join("career.ini")
}

fn load_career() -> Career {
    let mut c = Career::default();
    let Ok(text) = std::fs::read_to_string(career_path()) else {
        return c;
    };
    for line in text.lines() {
        let (k, v) = line.split_once(' ').unwrap_or((line, ""));
        match k {
            "campaign" => c.campaign = v.to_owned(),
            "stage" => c.stage = v.parse().unwrap_or(0),
            "flown" => c.flown = v.parse().unwrap_or(0),
            "won" => c.won = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    c
}

fn save_career(c: &Career) {
    let p = career_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(
        p,
        format!(
            "campaign {}\nstage {}\nflown {}\nwon {}\n",
            c.campaign, c.stage, c.flown, c.won
        ),
    );
}

/// QMB choices.
struct Quick {
    red: bool,
    map: usize,
    template: usize,
    ours: usize,
    our_num: u32,
    our_skill: u8,
    enemy: usize,
    enemy_num: u32,
    enemy_skill: u8,
    altitude: usize,
    hour: u32,
}

const ALTITUDES: [u32; 9] = [100, 500, 1000, 1500, 2000, 3000, 4000, 5000, 7000];

pub struct Menu {
    root: PathBuf,
    vfs: Arc<Vfs>,
    gui: HashMap<String, String>,
    single: Vec<Group>,
    planes: Vec<Plane>,
    templates: Vec<Template>,
    maps: Vec<String>,
    screen: Screen,
    sel: [usize; 3],
    scroll: [usize; 3],
    quick: Quick,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    hud: Hud,
    cursor: (f32, f32),
    clicked: bool,
    wheel: f32,
    child: Option<Child>,
    status: String,
    campaigns: Vec<Group>,
    career: Career,
    /// The result file of the running flight, and whether it is a career mission.
    pending: Option<(PathBuf, bool)>,
    dynamic: Dynamic,
    /// Per aircraft class: side, fighter, a loadout with bombs.
    plane_info: std::cell::RefCell<HashMap<String, (u8, bool, Option<String>)>>,
    fms: Option<il2_data::FlightModels>,
}

impl Menu {
    pub fn new(root: PathBuf, vfs: Vfs) -> Self {
        let lang = language();
        let gui = properties_vfs(&vfs, "i18n/gui", &lang);
        let single = single_missions(&root, &lang);
        let root_for_campaigns = root.clone();
        let planes = planes(&vfs, &lang);
        let templates = templates(&root);
        let mut maps: Vec<String> = templates.iter().map(|t| t.map.clone()).collect();
        maps.dedup();
        let pick = |class: &str| planes.iter().position(|p| p.class == class).unwrap_or(0);
        let quick = Quick {
            red: false,
            map: maps.iter().position(|m| m == "Kuban").unwrap_or(0),
            template: 0,
            ours: pick("air.BF_109G6"),
            our_num: 2,
            our_skill: 2,
            enemy: pick("air.LAGG_3SERIES4"),
            enemy_num: 2,
            enemy_skill: 1,
            altitude: 4,
            hour: 12,
        };
        Menu {
            hud: Hud::new(&root.join("lib/fonts/LucidaSansRegular.ttf")),
            root,
            vfs: Arc::new(vfs),
            gui,
            single,
            planes,
            templates,
            maps,
            screen: Screen::Main,
            sel: [0; 3],
            scroll: [0; 3],
            quick,
            window: None,
            renderer: None,
            cursor: (0.0, 0.0),
            clicked: false,
            wheel: 0.0,
            child: None,
            status: String::new(),
            campaigns: campaigns(&root_for_campaigns, &lang),
            career: load_career(),
            pending: None,
            dynamic: {
                let mut d = Dynamic {
                    theaters: crate::dgen::theaters(&root_for_campaigns),
                    ..Default::default()
                };
                if let Ok(t) = std::fs::read_to_string(dgen_path()) {
                    for l in t.lines() {
                        let (k, v) = l.split_once(' ').unwrap_or((l, ""));
                        match k {
                            "sorties" => d.sorties = v.parse().unwrap_or(0),
                            "won" => d.won = v.parse().unwrap_or(0),
                            _ => {}
                        }
                    }
                }
                d
            },
            plane_info: Default::default(),
            fms: None,
        }
    }

    fn s(&self, key: &str, fallback: &str) -> String {
        self.gui
            .get(key)
            .cloned()
            .unwrap_or_else(|| fallback.to_owned())
    }

    /// Starts a flight with these arguments as a child process; the menu hides until it ends.
    fn launch(&mut self, args: &[String]) {
        self.launch_with(args, false);
    }

    fn launch_with(&mut self, args: &[String], career: bool) {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("openil2"));
        let result = std::env::temp_dir().join("openil2_result.txt");
        let _ = std::fs::remove_file(&result);
        self.pending = Some((result.clone(), career));
        // Every flight is recorded; the last 20 tracks are kept.
        let dir = data_dir().join("tracks");
        let _ = std::fs::create_dir_all(&dir);
        for old in tracks().into_iter().skip(19) {
            let _ = std::fs::remove_file(old);
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut cmd = Command::new(exe);
        cmd.arg("--root")
            .arg(&self.root)
            .arg("--result")
            .arg(&result)
            .args(args);
        if !args.iter().any(|a| a == "--play") {
            cmd.arg("--record").arg(dir.join(format!("{stamp}.otrk")));
        }
        match cmd.spawn() {
            Ok(c) => {
                self.child = Some(c);
                if let Some(w) = &self.window {
                    w.set_visible(false);
                }
            }
            Err(e) => self.status = format!("cannot start the flight: {e}"),
        }
    }

    fn button(&mut self, x: f32, y: f32, w: f32, label: &str) -> bool {
        let h = self.hud.line_height() + 10.0;
        let (cx, cy) = self.cursor;
        let hover = cx >= x && cx <= x + w && cy >= y && cy <= y + h;
        self.hud.rect(
            x,
            y,
            x + w,
            y + h,
            if hover {
                [0.85, 0.65, 0.25, 0.9]
            } else {
                [0.18, 0.2, 0.17, 0.85]
            },
        );
        let tw = self.hud.text_width(label);
        self.hud
            .text(x + (w - tw) * 0.5, y + 5.0, label, [0.97, 0.95, 0.85, 1.0]);
        hover && self.clicked
    }

    /// A scrolling list; returns the clicked index.
    #[allow(clippy::too_many_arguments)]
    fn list(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        rows: usize,
        items: &[String],
        selected: usize,
        slot: usize,
    ) -> Option<usize> {
        let lh = self.hud.line_height() + 4.0;
        let (cx, cy) = self.cursor;
        let inside = cx >= x && cx <= x + w && cy >= y && cy <= y + lh * rows as f32;
        if inside && self.wheel != 0.0 {
            let s = self.scroll[slot] as i64 - self.wheel.signum() as i64 * 3;
            self.scroll[slot] = s.clamp(0, items.len().saturating_sub(rows) as i64) as usize;
        }
        self.hud
            .rect(x, y, x + w, y + lh * rows as f32, [0.08, 0.09, 0.08, 0.75]);
        let mut hit = None;
        for (k, item) in items.iter().enumerate().skip(self.scroll[slot]).take(rows) {
            let ry = y + lh * (k - self.scroll[slot]) as f32;
            let hover = inside && cy >= ry && cy < ry + lh;
            if k == selected {
                self.hud
                    .rect(x, ry, x + w, ry + lh, [0.55, 0.42, 0.18, 0.9]);
            } else if hover {
                self.hud.rect(x, ry, x + w, ry + lh, [0.3, 0.3, 0.25, 0.8]);
            }
            let mut text = item.clone();
            while self.hud.text_width(&text) > w - 12.0 && text.chars().count() > 3 {
                text.pop();
            }
            self.hud
                .text(x + 6.0, ry + 2.0, &text, [0.95, 0.93, 0.85, 1.0]);
            if hover && self.clicked {
                hit = Some(k);
            }
        }
        hit
    }

    /// `label   < value >`; returns −1, 0 or +1.
    fn selector(&mut self, x: f32, y: f32, label: &str, value: &str) -> i32 {
        self.hud.text(x, y + 5.0, label, [0.9, 0.85, 0.65, 1.0]);
        let mut d = 0;
        if self.button(x + 230.0, y, 34.0, "<") {
            d = -1;
        }
        self.hud
            .text(x + 274.0, y + 5.0, value, [0.97, 0.95, 0.85, 1.0]);
        if self.button(x + 640.0, y, 34.0, ">") {
            d = 1;
        }
        d
    }

    fn wrap(&self, text: &str, width: f32) -> Vec<String> {
        let mut out = Vec::new();
        for para in text.split('\n') {
            let mut line = String::new();
            for word in para.split_whitespace() {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if self.hud.text_width(&candidate) > width && !line.is_empty() {
                    out.push(std::mem::replace(&mut line, word.to_owned()));
                } else {
                    line = candidate;
                }
            }
            out.push(line);
        }
        out
    }

    fn draw(&mut self, w: f32, h: f32) {
        self.hud.begin();
        self.hud.rect(0.0, 0.0, w, h, [0.12, 0.14, 0.12, 1.0]);
        self.hud.rect(0.0, 0.0, w, 56.0, [0.06, 0.07, 0.06, 1.0]);
        let title = match self.screen {
            Screen::Main => self.s("main.info", "Main Menu"),
            Screen::Single => self.s("single.info", "Single Missions"),
            Screen::Quick => self.s("quick.info", "Quick Mission Builder"),
            Screen::Career => self.s("main.PilotCareer", "Pilot Career"),
            Screen::Tracks => self.s("main.PlayTrack", "Play Track"),
            Screen::Dynamic => "Dynamic Campaign".to_owned(),
        };
        self.hud.text(
            24.0,
            16.0,
            &format!("OpenIL2  ·  {title}"),
            [0.95, 0.8, 0.45, 1.0],
        );
        match self.screen {
            Screen::Main => self.draw_main(w, h),
            Screen::Single => self.draw_single(w, h),
            Screen::Quick => self.draw_quick(w, h),
            Screen::Career => self.draw_career(w, h),
            Screen::Tracks => self.draw_tracks(w, h),
            Screen::Dynamic => self.draw_dynamic(w, h),
        }
        if !self.status.is_empty() {
            let s = self.status.clone();
            self.hud.text(24.0, h - 36.0, &s, [1.0, 0.6, 0.4, 1.0]);
        }
    }

    fn draw_main(&mut self, _w: f32, h: f32) {
        let items = [
            ("main.SingleMissions", "Single Missions"),
            ("main.PilotCareer", "Pilot Career"),
            ("main.DynamicCampaign", "Dynamic Campaign"),
            ("main.Quick", "Quick Mission Builder"),
            ("main.PlayTrack", "Play Track"),
            ("main.Quit", "Quit"),
        ];
        let mut y = h * 0.3;
        for (i, (k, f)) in items.iter().enumerate() {
            let label = self.s(k, f);
            if self.button(80.0, y, 360.0, &label) {
                match i {
                    0 => self.screen = Screen::Single,
                    1 => {
                        self.screen = Screen::Career;
                        self.sel = [0; 3];
                        self.scroll = [0; 3];
                    }
                    2 => {
                        self.screen = Screen::Dynamic;
                        self.scroll = [0; 3];
                    }
                    3 => self.screen = Screen::Quick,
                    4 => {
                        self.screen = Screen::Tracks;
                        self.sel = [0; 3];
                        self.scroll = [0; 3];
                    }
                    _ => std::process::exit(0),
                }
            }
            y += self.hud.line_height() + 24.0;
        }
    }

    fn draw_single(&mut self, w: f32, h: f32) {
        let rows = ((h - 200.0) / (self.hud.line_height() + 4.0)) as usize;
        let countries: Vec<String> = self.single.iter().map(|g| g.name.clone()).collect();
        if let Some(i) = self.list(24.0, 80.0, 120.0, rows, &countries, self.sel[0], 0) {
            self.sel = [i, 0, 0];
            self.scroll[1] = 0;
            self.scroll[2] = 0;
        }
        let Some(c) = self.single.get(self.sel[0]) else {
            return;
        };
        let planes: Vec<String> = c.items.iter().map(|g| g.name.clone()).collect();
        if let Some(i) = self.list(156.0, 80.0, 260.0, rows, &planes, self.sel[1], 1) {
            self.sel[1] = i;
            self.sel[2] = 0;
            self.scroll[2] = 0;
        }
        let Some(p) = self.single[self.sel[0]].items.get(self.sel[1]) else {
            return;
        };
        let missions: Vec<String> = p.missions.iter().map(|m| m.name.clone()).collect();
        if let Some(i) = self.list(428.0, 80.0, 360.0, rows, &missions, self.sel[2], 2) {
            self.sel[2] = i;
        }
        let Some(m) = self.single[self.sel[0]].items[self.sel[1]]
            .missions
            .get(self.sel[2])
        else {
            return;
        };
        let (path, briefing) = (m.path.clone(), m.briefing.clone());
        let bx = 804.0;
        let lines = self.wrap(&briefing, (w - bx - 24.0).max(200.0));
        let mut y = 84.0;
        for l in lines.iter().take(rows + 2) {
            self.hud.text(bx, y, l, [0.9, 0.9, 0.82, 1.0]);
            y += self.hud.line_height();
        }
        let by = h - 80.0;
        if self.button(24.0, by, 200.0, &self.s("quick.BAC", "Back")) {
            self.screen = Screen::Main;
        }
        if self.button(w - 224.0, by, 200.0, &self.s("quick.FLY", "Fly")) {
            self.launch(&["--mission".into(), path]);
        }
    }

    fn draw_career(&mut self, w: f32, h: f32) {
        let rows = ((h - 200.0) / (self.hud.line_height() + 4.0)) as usize;
        let countries: Vec<String> = self.campaigns.iter().map(|g| g.name.clone()).collect();
        if let Some(i) = self.list(24.0, 80.0, 120.0, rows, &countries, self.sel[0], 0) {
            self.sel = [i, 0, 0];
            self.scroll[1] = 0;
        }
        let Some(c) = self.campaigns.get(self.sel[0]) else {
            return;
        };
        let names: Vec<String> = c.items.iter().map(|g| g.name.clone()).collect();
        if let Some(i) = self.list(156.0, 80.0, 360.0, rows, &names, self.sel[1], 1) {
            self.sel[1] = i;
        }
        let Some(camp) = self.campaigns[self.sel[0]].items.get(self.sel[1]) else {
            return;
        };
        let (dir, about, stages) = (
            camp.missions[0].path.clone(),
            camp.missions[0].briefing.clone(),
            camp.stages.clone(),
        );
        let ours = self.career.campaign == dir;
        let stage = if ours { self.career.stage } else { 0 };
        let bx = 532.0;
        let mut y = 84.0;
        let progress = if stage >= stages.len() {
            "Campaign complete".to_owned()
        } else if ours {
            format!(
                "Mission {} of {}   (flown {}, successful {})",
                stage + 1,
                stages.len(),
                self.career.flown,
                self.career.won
            )
        } else {
            format!("{} missions", stages.len())
        };
        self.hud.text(bx, y, &progress, [0.95, 0.8, 0.45, 1.0]);
        y += self.hud.line_height() * 1.5;
        for l in self
            .wrap(&about, (w - bx - 24.0).max(200.0))
            .iter()
            .take(rows)
        {
            self.hud.text(bx, y, l, [0.9, 0.9, 0.82, 1.0]);
            y += self.hud.line_height();
        }
        let by = h - 80.0;
        if self.button(24.0, by, 200.0, &self.s("quick.BAC", "Back")) {
            self.screen = Screen::Main;
        }
        let label = if ours && stage > 0 {
            "Continue"
        } else {
            "Start"
        };
        if stage < stages.len() && self.button(w - 224.0, by, 200.0, label) {
            if !ours {
                self.career = Career {
                    campaign: dir.clone(),
                    ..Default::default()
                };
                save_career(&self.career);
            }
            let alts = &stages[stage];
            let pick = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis()) as usize)
                % alts.len().max(1);
            let mission = format!("{dir}/{}", alts[pick]);
            self.launch_with(&["--mission".into(), mission], true);
        }
    }

    /// Side, fighter, bomb loadout of an aircraft class (cached; needs the flight models' keys).
    fn info(&mut self, class: &str) -> (u8, bool, Option<String>) {
        if let Some(i) = self.plane_info.borrow().get(class) {
            return i.clone();
        }
        if self.fms.is_none() {
            self.fms = il2_data::FlightModels::open(&self.vfs).ok();
        }
        let army = self
            .planes
            .iter()
            .find(|p| p.class.eq_ignore_ascii_case(class))
            .map_or(0, |p| p.army);
        let (fighter, bombs) = match &self.fms {
            Some(f) => {
                let c = il2_data::Classes::new(&self.vfs, &f.keys);
                let ifs = c.interfaces(class);
                let bombs = c.aircraft_weapons(class).and_then(|w| {
                    w.loadouts
                        .iter()
                        .find(|(_, slots)| {
                            slots
                                .iter()
                                .flatten()
                                .any(|(g, _)| g.starts_with("BombGun"))
                        })
                        .map(|(n, _)| n.clone())
                });
                (ifs.iter().any(|i| i == "TypeFighter"), bombs)
            }
            None => (true, None),
        };
        let i = (army, fighter, bombs);
        self.plane_info
            .borrow_mut()
            .insert(class.to_owned(), i.clone());
        i
    }

    fn draw_dynamic(&mut self, w: f32, h: f32) {
        let rows = ((h - 200.0) / (self.hud.line_height() + 4.0)) as usize;
        let names = self.dynamic.theaters.clone();
        if let Some(i) = self.list(24.0, 80.0, 220.0, rows, &names, self.dynamic.sel, 0) {
            self.dynamic.sel = i;
            self.dynamic.loaded = None;
            self.dynamic.plane = 0;
        }
        let Some(name) = names.get(self.dynamic.sel).cloned() else {
            return;
        };
        if self.dynamic.loaded.as_ref().is_none_or(|t| t.name != name) {
            match crate::dgen::load(&self.root, &name) {
                Ok(t) => self.dynamic.loaded = Some(t),
                Err(e) => {
                    self.status = e;
                    return;
                }
            }
        }
        let army = if self.dynamic.red { 1 } else { 2 };
        let classes: Vec<String> = self
            .dynamic
            .loaded
            .as_ref()
            .map(|t| t.planes.iter().map(|p| format!("air.{}", p.1)).collect())
            .unwrap_or_default();
        let ours: Vec<String> = classes
            .into_iter()
            .filter(|c| {
                self.planes
                    .iter()
                    .any(|p| p.class.eq_ignore_ascii_case(c) && p.army == army)
            })
            .collect();
        let x = 270.0;
        let side = if self.dynamic.red {
            self.s("quick.SEL_Allies:", "Allies")
        } else {
            self.s("quick.SEL_Axis:", "Axis")
        };
        if self.selector(x, 80.0, &self.s("quick.SEL:", "Side:"), &side) != 0 {
            self.dynamic.red = !self.dynamic.red;
            self.dynamic.plane = 0;
        }
        let shown: Vec<String> = ours
            .iter()
            .map(|c| {
                self.planes
                    .iter()
                    .find(|p| p.class.eq_ignore_ascii_case(c))
                    .map_or(c.clone(), |p| p.name.clone())
            })
            .collect();
        if let Some(i) = self.list(
            x,
            130.0,
            420.0,
            rows.saturating_sub(3),
            &shown,
            self.dynamic.plane,
            1,
        ) {
            self.dynamic.plane = i;
        }
        let map = self
            .dynamic
            .loaded
            .as_ref()
            .map_or(String::new(), |t| t.map.clone());
        let lines = [
            format!("Map: {map}"),
            format!(
                "Sorties flown: {}   successful: {}",
                self.dynamic.sorties, self.dynamic.won
            ),
            self.dynamic.last.clone(),
        ];
        let mut y = 130.0;
        for l in lines {
            self.hud.text(x + 440.0, y, &l, [0.9, 0.9, 0.82, 1.0]);
            y += self.hud.line_height() * 1.4;
        }
        let by = h - 80.0;
        if self.button(24.0, by, 200.0, &self.s("quick.BAC", "Back")) {
            self.screen = Screen::Main;
        }
        let Some(class) = ours.get(self.dynamic.plane).cloned() else {
            return;
        };
        if self.button(w - 224.0, by, 200.0, &self.s("quick.FLY", "Fly")) {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(7, |d| d.as_nanos() as u64);
            // Look the aircraft data up before generating (the generator asks for it).
            if let Some(t) = &self.dynamic.loaded {
                let all: Vec<String> = t.planes.iter().map(|p| format!("air.{}", p.1)).collect();
                for c in all {
                    self.info(&c);
                }
            }
            self.info(&class);
            let infos = self.plane_info.borrow().clone();
            let lookup = move |c: &str| infos.get(c).cloned().unwrap_or((0, true, None));
            let world = crate::dgen::World {
                vfs: &self.vfs,
                root: &self.root,
                plane_info: &lookup,
            };
            let t = self.dynamic.loaded.as_ref().expect("loaded");
            match crate::dgen::generate(&world, t, army, &class, self.dynamic.sorties, seed) {
                Ok((text, summary)) => {
                    let path = std::env::temp_dir().join("openil2_dynamic.mis");
                    let bytes: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
                    match std::fs::write(&path, bytes) {
                        Ok(()) => {
                            self.dynamic.last = summary;
                            self.launch(&["--mission".into(), path.to_string_lossy().into_owned()]);
                        }
                        Err(e) => self.status = e.to_string(),
                    }
                }
                Err(e) => self.status = e,
            }
        }
    }

    fn draw_tracks(&mut self, w: f32, h: f32) {
        let rows = ((h - 200.0) / (self.hud.line_height() + 4.0)) as usize;
        let list = tracks();
        let names: Vec<String> = list
            .iter()
            .map(|p| {
                let secs: u64 = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let opts = std::fs::read(p).ok().and_then(|d| {
                    // The options right after the header: show the mission or aircraft.
                    let text = String::from_utf8_lossy(&d[8..d.len().min(600)]).into_owned();
                    text.split(|c: char| c.is_control())
                        .filter(|s| s.ends_with(".mis") || s.starts_with("air."))
                        .last()
                        .map(str::to_owned)
                });
                let ago = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
                    .saturating_sub(secs);
                format!("{} min ago   {}", ago / 60, opts.unwrap_or_default())
            })
            .collect();
        if let Some(i) = self.list(24.0, 80.0, w - 48.0, rows, &names, self.sel[0], 0) {
            self.sel[0] = i;
        }
        let by = h - 80.0;
        if self.button(24.0, by, 200.0, &self.s("quick.BAC", "Back")) {
            self.screen = Screen::Main;
        }
        if let Some(p) = list.get(self.sel[0]) {
            if self.button(
                w - 224.0,
                by,
                200.0,
                &self.s("main.PlayTrack", "Play Track"),
            ) {
                let p = p.to_string_lossy().into_owned();
                self.launch(&["--play".into(), p]);
            }
        }
    }

    fn templates_for(&self) -> Vec<usize> {
        let map = self.maps.get(self.quick.map).cloned().unwrap_or_default();
        (0..self.templates.len())
            .filter(|&i| self.templates[i].map == map && self.templates[i].red == self.quick.red)
            .collect()
    }

    fn draw_quick(&mut self, w: f32, h: f32) {
        let q = &self.quick;
        let side = if q.red {
            self.s("quick.SEL_Allies:", "Allies")
        } else {
            self.s("quick.SEL_Axis:", "Axis")
        };
        let skill = |s: u8| ["Rookie", "Average", "Veteran", "Ace"][s.min(3) as usize].to_owned();
        let tl = self.templates_for();
        let (x, mut y) = (60.0, 90.0);
        let step = self.hud.line_height() + 18.0;
        let wrapi = |v: usize, d: i32, n: usize| {
            if n == 0 {
                0
            } else {
                ((v as i64 + d as i64).rem_euclid(n as i64)) as usize
            }
        };
        let our_army = if self.quick.red { 1 } else { 2 };
        let ours: Vec<usize> = (0..self.planes.len())
            .filter(|&i| self.planes[i].army == our_army)
            .collect();
        let theirs: Vec<usize> = (0..self.planes.len())
            .filter(|&i| self.planes[i].army != our_army)
            .collect();
        let pos_in = |list: &[usize], v: usize| list.iter().position(|&i| i == v).unwrap_or(0);

        let d = self.selector(x, y, &self.s("quick.SEL:", "Side:"), &side);
        if d != 0 {
            self.quick.red = !self.quick.red;
            self.quick.template = 0;
            let army = if self.quick.red { 1 } else { 2 };
            std::mem::swap(&mut self.quick.ours, &mut self.quick.enemy);
            if self
                .planes
                .get(self.quick.ours)
                .is_some_and(|p| p.army != army)
            {
                self.quick.ours = (0..self.planes.len())
                    .find(|&i| self.planes[i].army == army)
                    .unwrap_or(0);
            }
        }
        y += step;
        let map_name = self.maps.get(self.quick.map).cloned().unwrap_or_default();
        let d = self.selector(x, y, &self.s("quick.MAP:", "Map:"), &map_name);
        if d != 0 {
            self.quick.map = wrapi(self.quick.map, d, self.maps.len());
            self.quick.template = 0;
        }
        y += step;
        let sit = tl
            .get(self.quick.template)
            .map(|&i| self.templates[i].situation.clone())
            .unwrap_or_default();
        let d = self.selector(x, y, &self.s("quick.SIT:", "Situation:"), &sit);
        if d != 0 {
            self.quick.template = wrapi(self.quick.template, d, tl.len());
        }
        y += step * 1.5;
        let our_name = self
            .planes
            .get(self.quick.ours)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let d = self.selector(x, y, &self.s("quick.YOU:", "Your Flight:"), &our_name);
        if d != 0 {
            self.quick.ours = ours[wrapi(pos_in(&ours, self.quick.ours), d, ours.len())];
        }
        y += step;
        let d = self.selector(
            x,
            y,
            &format!("  {}", self.s("quick.NUM:", "Num.")),
            &self.quick.our_num.to_string(),
        );
        self.quick.our_num = (self.quick.our_num as i32 + d).clamp(1, 4) as u32;
        y += step;
        let d = self.selector(
            x,
            y,
            &format!("  {}", self.s("quick.SKI:", "Skill")),
            &skill(self.quick.our_skill),
        );
        self.quick.our_skill = (self.quick.our_skill as i32 + d).clamp(0, 3) as u8;
        y += step * 1.5;
        let their_name = self
            .planes
            .get(self.quick.enemy)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let d = self.selector(x, y, &self.s("quick.ENM:", "Hostile:"), &their_name);
        if d != 0 {
            self.quick.enemy = theirs[wrapi(pos_in(&theirs, self.quick.enemy), d, theirs.len())];
        }
        y += step;
        let d = self.selector(
            x,
            y,
            &format!("  {}", self.s("quick.NUM:", "Num.")),
            &self.quick.enemy_num.to_string(),
        );
        self.quick.enemy_num = (self.quick.enemy_num as i32 + d).clamp(0, 4) as u32;
        y += step;
        let d = self.selector(
            x,
            y,
            &format!("  {}", self.s("quick.SKI:", "Skill")),
            &skill(self.quick.enemy_skill),
        );
        self.quick.enemy_skill = (self.quick.enemy_skill as i32 + d).clamp(0, 3) as u8;
        y += step * 1.5;
        let d = self.selector(
            x,
            y,
            &self.s("quick.ALT:", "Altitude"),
            &format!("{} m", ALTITUDES[self.quick.altitude]),
        );
        self.quick.altitude =
            (self.quick.altitude as i32 + d).clamp(0, ALTITUDES.len() as i32 - 1) as usize;
        y += step;
        let d = self.selector(
            x,
            y,
            &self.s("quick.TIM:", "Time"),
            &format!("{:02}:00", self.quick.hour),
        );
        self.quick.hour = (self.quick.hour as i32 + d).clamp(5, 20) as u32;

        let by = h - 80.0;
        if self.button(24.0, by, 200.0, &self.s("quick.BAC", "Back")) {
            self.screen = Screen::Main;
        }
        if self.button(w - 224.0, by, 200.0, &self.s("quick.FLY", "Fly")) {
            match tl.get(self.quick.template).map(|&i| self.write_quick(i)) {
                Some(Ok(path)) => {
                    self.launch(&["--mission".into(), path.to_string_lossy().into_owned()])
                }
                Some(Err(e)) => self.status = e,
                None => self.status = "no template for this map and side".into(),
            }
        }
    }

    /// Fills the template like the runtime's QMB: the player's flight `r0100`/`g0100` and the
    /// first hostile flight get the chosen aircraft, counts and skills; other flights are removed;
    /// waypoint heights take the chosen altitude.
    fn write_quick(&self, t: usize) -> Result<PathBuf, String> {
        let tpl = &self.templates[t];
        let text = std::fs::read(&tpl.path).map_err(|e| e.to_string())?;
        let text: String = text.iter().map(|&b| b as char).collect();
        let q = &self.quick;
        let (us, them) = if q.red {
            ("r01", "g01")
        } else {
            ("g01", "r01")
        };
        let our_wing = format!("{us}00");
        let their_wing = format!("{them}00");
        let plane = |i: usize| {
            self.planes
                .get(i)
                .map(|p| p.class.clone())
                .unwrap_or_default()
        };
        let alt = ALTITUDES[q.altitude];
        let mut out = String::new();
        let mut section = String::new();
        let mut wings: Vec<String> = Vec::new();
        let mut drop = false;
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with('[') && l.ends_with(']') {
                section = l[1..l.len() - 1].to_owned();
                let base = section.strip_suffix("_Way").unwrap_or(&section).to_owned();
                let is_wing = wings.contains(&base);
                drop = is_wing && base != our_wing && !(base == their_wing && q.enemy_num > 0);
                if drop {
                    continue;
                }
                out += line;
                out.push('\n');
                if is_wing && !section.ends_with("_Way") {
                    let (class, num, skill) = if base == our_wing {
                        (plane(q.ours), q.our_num, q.our_skill)
                    } else {
                        (plane(q.enemy), q.enemy_num, q.enemy_skill)
                    };
                    out += &format!("  Planes {num}\n  Skill {skill}\n  Class {class}\n  Fuel 100\n  weapons default\n");
                    drop = true; // the template's own wing lines are replaced
                    section.clear();
                    continue;
                }
                continue;
            }
            if section == "Wing" {
                if !l.is_empty() {
                    wings.push(l.to_owned());
                    if l != our_wing && !(l == their_wing && q.enemy_num > 0) {
                        continue;
                    }
                }
            } else if drop {
                continue;
            } else if section == "MAIN" && l.starts_with("TIME") {
                out += &format!("  TIME {}.0\n", q.hour);
                continue;
            } else if section == "MAIN" && l.starts_with("player ") {
                out += &format!("  player {our_wing}\n");
                continue;
            } else if section.ends_with("_Way") && !l.is_empty() {
                let mut t: Vec<String> = l.split_whitespace().map(str::to_owned).collect();
                if t.len() >= 4
                    && !t[0].starts_with("TAKEOFF")
                    && !t[0].starts_with("LANDING")
                    && !t[0].starts_with("TRIGGERS")
                {
                    t[3] = format!("{alt}.00");
                }
                out += &format!("  {}\n", t.join(" "));
                continue;
            }
            out += line;
            out.push('\n');
        }
        if !out.contains("\n  player ") {
            out = out.replacen("[MAIN]\n", &format!("[MAIN]\n  player {our_wing}\n"), 1);
        }
        let path = std::env::temp_dir().join("openil2_quick.mis");
        let bytes: Vec<u8> = out.chars().map(|c| c as u32 as u8).collect();
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

impl ApplicationHandler for Menu {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let window = Arc::new(
            el.create_window(Window::default_attributes().with_title("OpenIL2"))
                .expect("window"),
        );
        let mut r = Renderer::new(window.clone());
        if let Some(a) = &self.hud.atlas {
            r.set_overlay_atlas(a);
        }
        self.renderer = Some(r);
        self.window = Some(window);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(c) = self.child.as_mut() {
            if let Ok(Some(status)) = c.try_wait() {
                if !status.success() {
                    self.status = format!("the flight ended with {status}");
                }
                self.child = None;
                self.finish_flight();
                if let Some(w) = &self.window {
                    w.set_visible(true);
                    w.request_redraw();
                }
            }
            el.set_control_flow(ControlFlow::WaitUntil(
                std::time::Instant::now() + std::time::Duration::from_millis(200),
            ));
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(s.width, s.height);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                self.request();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.clicked = true;
                self.request();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.wheel = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as f32,
                };
                self.request();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                {
                    if self.screen == Screen::Main {
                        el.exit();
                    } else {
                        self.screen = Screen::Main;
                    }
                    self.request();
                }
            }
            WindowEvent::RedrawRequested => {
                let Some((w, h)) = self
                    .renderer
                    .as_ref()
                    .map(|r| (r.size().0 as f32, r.size().1 as f32))
                else {
                    return;
                };
                self.draw(w, h);
                self.clicked = false;
                self.wheel = 0.0;
                let r = self.renderer.as_mut().expect("renderer");
                r.set_overlay(self.hud.finish());
                r.render(glam::Mat4::IDENTITY, 1.0, None, &[], &[], None);
            }
            _ => {}
        }
    }
}

impl Menu {
    /// Reads the finished flight's result: shows it, and moves a career on after a success.
    fn finish_flight(&mut self) {
        let Some((path, career)) = self.pending.take() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let success = text.lines().any(|l| l == "result success");
        self.status = if success {
            self.s("single.Complete", "Mission Complete")
        } else {
            self.s("single.Failed", "Mission Failed")
        };
        if self.screen == Screen::Dynamic {
            self.dynamic.sorties += 1;
            self.dynamic.won += success as u32;
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(
                dgen_path(),
                format!(
                    "sorties {}\nwon {}\n",
                    self.dynamic.sorties, self.dynamic.won
                ),
            );
        }
        if career {
            self.career.flown += 1;
            if success {
                self.career.won += 1;
                self.career.stage += 1;
            }
            save_career(&self.career);
        }
    }

    fn request(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

/// The menu's own entry point.
pub fn run(root: PathBuf) {
    let vfs = Vfs::open_install(&root).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1)
    });
    let mut menu = Menu::new(root, vfs);
    let el = winit::event_loop::EventLoop::new().expect("event loop");
    el.run_app(&mut menu).expect("run");
    let _ = &menu.vfs;
}

/// Renders one menu screen (`main`, `single`, `quick`) offscreen to a PNG (tests, docs).
pub fn screenshot(root: PathBuf, screen: &str, out: &Path) {
    let vfs = Vfs::open_install(&root).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1)
    });
    let mut menu = Menu::new(root, vfs);
    if let Some(spec) = screen.strip_prefix("write-dynamic:") {
        // `write-dynamic:Theater,red|blue,air.CLASS`: generates a sortie and prints its path.
        let mut it = spec.split(',');
        let (th, side, class) = (
            it.next().unwrap_or("Bagration"),
            it.next().unwrap_or("red"),
            it.next().unwrap_or("air.YAK_9D"),
        );
        let t = crate::dgen::load(&menu.root, th).expect("theater");
        let classes: Vec<String> = t
            .planes
            .iter()
            .map(|p| format!("air.{}", p.1))
            .chain([class.to_owned()])
            .collect();
        for c in classes {
            menu.info(&c);
        }
        let infos = menu.plane_info.borrow().clone();
        let lookup = move |c: &str| infos.get(c).cloned().unwrap_or((0, true, None));
        let world = crate::dgen::World {
            vfs: &menu.vfs,
            root: &menu.root,
            plane_info: &lookup,
        };
        match crate::dgen::generate(
            &world,
            &t,
            if side == "red" { 1 } else { 2 },
            class,
            0,
            12345,
        ) {
            Ok((text, summary)) => {
                let path = std::env::temp_dir().join("openil2_dynamic.mis");
                std::fs::write(
                    &path,
                    text.chars().map(|c| c as u32 as u8).collect::<Vec<u8>>(),
                )
                .expect("write");
                println!("{}\n{summary}", path.display());
            }
            Err(e) => eprintln!("{e}"),
        }
        return;
    }
    if screen == "write-quick" {
        // Writes the default quick mission and prints its path.
        let t = menu.templates_for();
        match t.first().map(|&i| menu.write_quick(i)) {
            Some(Ok(p)) => println!("{}", p.display()),
            other => eprintln!("no quick mission: {:?}", other.map(|r| r.err())),
        }
        return;
    }
    menu.screen = match screen {
        "single" => Screen::Single,
        "quick" => Screen::Quick,
        "career" => Screen::Career,
        "dynamic" => Screen::Dynamic,
        _ => Screen::Main,
    };
    let (w, h) = (1280, 720);
    let mut r = Renderer::headless(w, h);
    if let Some(a) = &menu.hud.atlas {
        r.set_overlay_atlas(a);
    }
    menu.cursor = (300.0, 320.0);
    menu.draw(w as f32, h as f32);
    r.set_overlay(menu.hud.finish());
    let px = r.screenshot(glam::Mat4::IDENTITY, 1.0, None, &[], &[], None);
    image::save_buffer(out, &px, w, h, image::ExtendedColorType::Rgba8).expect("png");
}
