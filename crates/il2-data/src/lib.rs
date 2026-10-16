//! Game data that the runtime keeps scrambled: flight models and engine definitions.
//! Formats: `docs/formats/krypto.md`. All key material is read from the user's install.

pub mod anim;
mod classfile;
pub mod container;
pub mod keys;
mod pe;

use il2_formats::SectFile;
use il2_sfs::int_hash;
use il2_vfs::Vfs;
use std::collections::HashMap;

pub use keys::KeyTable;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Keys(#[from] keys::Error),
    #[error(transparent)]
    Vfs(#[from] il2_vfs::Error),
    #[error(transparent)]
    Container(#[from] container::Error),
    #[error("{0}: not in the flight-model container")]
    NotFound(String),
}

/// What an aircraft class says about its assets.
#[derive(Clone, Debug)]
pub struct AircraftClass {
    pub flight_model: String,
    /// Hierarchical mesh paths; the default (no country suffix) first.
    pub meshes: Vec<String>,
}

/// Weapon slots of an aircraft: per slot a trigger group and a hook, and named loadouts that put a
/// weapon class and ammunition count (or nothing) in each slot.
#[derive(Clone, Debug, Default)]
pub struct AircraftWeapons {
    pub triggers: Vec<i32>,
    pub hooks: Vec<String>,
    pub loadouts: Vec<(String, Vec<Option<(String, i32)>>)>,
}

/// Reads runtime classes from the install, only to look up asset names (`docs/formats/mission.md`).
pub struct Classes<'a> {
    vfs: &'a Vfs,
    keys: &'a KeyTable,
}

impl<'a> Classes<'a> {
    pub fn new(vfs: &'a Vfs, keys: &'a KeyTable) -> Self {
        Classes { vfs, keys }
    }

    fn load(&self, name: &str) -> Option<classfile::ClassFile> {
        let h = int_hash(&format!("sdw{name}cwc2w9e"));
        let data = self.vfs.read(&format!("cod/{h}")).ok()?;
        let key = self.keys.class_key(h);
        let mut class = keys::CLASS_HEADER.to_vec();
        class.extend(data.iter().enumerate().map(|(i, b)| b ^ key[i % key.len()]));
        classfile::ClassFile::parse(&class).ok()
    }

    /// The class and its superclasses, most derived first, as far as they are stored in the install.
    fn chain(&self, full: &str) -> Vec<classfile::ClassFile> {
        let mut out = Vec::new();
        let mut name = Some(full.replace('/', "."));
        while let Some(n) = name.take() {
            if !n.starts_with("com.maddox") || out.len() > 16 {
                break;
            }
            let Some(c) = self.load(&n) else { break };
            name = c.super_class.as_ref().map(|s| s.replace('/', "."));
            out.push(c);
        }
        out
    }

    /// A class with its superclasses, plus (transitively) every class their code refers to, as far as
    /// they are stored in the install; the requested class comes first.
    fn pool(&self, full: &str) -> Vec<classfile::ClassFile> {
        let mut pool = self.chain(full);
        let mut i = 0;
        while i < pool.len() && pool.len() < 64 {
            let refs: Vec<String> = pool[i]
                .consts
                .iter()
                .filter_map(|c| match c {
                    classfile::Const::Class(n) => pool[i].utf8(*n).map(|s| s.replace('/', ".")),
                    _ => None,
                })
                .filter(|n| n.starts_with("com.maddox"))
                .collect();
            for r in refs {
                if !pool.iter().any(|c| c.this_class.replace('/', ".") == r) {
                    for c in self.chain(&r) {
                        if !pool.iter().any(|p| p.this_class == c.this_class) {
                            pool.push(c);
                        }
                    }
                }
            }
            i += 1;
        }
        pool
    }

    /// An aircraft class's weapon slots and loadouts, from its static initializer
    /// (`docs/behavior/weapons.md`).
    pub fn aircraft_weapons(&self, class: &str) -> Option<AircraftWeapons> {
        let full = if class.starts_with("com.") {
            class.to_owned()
        } else {
            format!("com.maddox.il2.objects.{class}")
        };
        let pool = self.pool(&full);
        if pool.is_empty() {
            return None;
        }
        let calls = anim::record_clinit(
            &pool,
            &[
                "weaponTriggersRegister",
                "weaponHooksRegister",
                "weaponsRegister",
            ],
        );
        let mut w = AircraftWeapons::default();
        for (name, args) in calls {
            match name.as_str() {
                "weaponTriggersRegister" => {
                    if let Some(anim::Value::List(l)) = args.get(1) {
                        w.triggers = l
                            .iter()
                            .map(|v| {
                                if let anim::Value::Num(n) = v {
                                    *n as i32
                                } else {
                                    0
                                }
                            })
                            .collect();
                    }
                }
                "weaponHooksRegister" => {
                    if let Some(anim::Value::List(l)) = args.get(1) {
                        w.hooks = l
                            .iter()
                            .map(|v| {
                                if let anim::Value::Str(s) = v {
                                    s.clone()
                                } else {
                                    String::new()
                                }
                            })
                            .collect();
                    }
                }
                _ => {
                    let (Some(anim::Value::Str(lname)), Some(anim::Value::List(l))) =
                        (args.get(1), args.get(2))
                    else {
                        continue;
                    };
                    let slots = l
                        .iter()
                        .map(|v| match v {
                            anim::Value::Str(s) => {
                                let mut t = s.split_whitespace();
                                let gun = t.next()?.to_owned();
                                let count = t.next().and_then(|c| c.parse().ok()).unwrap_or(0);
                                Some((gun, count))
                            }
                            _ => None,
                        })
                        .collect();
                    w.loadouts.push((lname.clone(), slots));
                }
            }
        }
        Some(w)
    }

    /// `Property.set(cls, key, value)` calls of a class's static initializer, for that class
    /// (e.g. bomb `mass`, `power`, `mesh`; bomb gun `bulletClass`, `bullets`, `shotFreq`).
    pub fn class_properties(&self, full: &str) -> HashMap<String, anim::Value> {
        let full = full.replace('/', ".");
        let pool = self.pool(&full);
        let mut out = HashMap::new();
        if pool.is_empty() {
            return out;
        }
        for (_, args) in anim::record_clinit(&pool, &["set"]) {
            let (Some(anim::Value::Str(cls)), Some(anim::Value::Str(key)), Some(v)) =
                (args.first(), args.get(1), args.get(2))
            else {
                continue;
            };
            if cls.replace('/', ".") == full {
                out.insert(key.clone(), v.clone());
            }
        }
        out
    }

    /// The pilot's cockpit of an aircraft class: the cockpit class and its hierarchy (`.him`).
    pub fn cockpit(&self, class: &str) -> Option<(String, String)> {
        let full = if class.starts_with("com.") {
            class.to_owned()
        } else {
            format!("com.maddox.il2.objects.{class}")
        };
        let names: Vec<String> = self
            .chain(&full)
            .iter()
            .map(|c| c.this_class.replace('/', "."))
            .collect();
        let cockpit =
            names
                .iter()
                .find_map(|n| match self.class_properties(n).get("cockpitClass")? {
                    anim::Value::Str(s) => Some(s.clone()),
                    anim::Value::List(l) => l.iter().find_map(|v| {
                        if let anim::Value::Str(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    }),
                    _ => None,
                })?;
        let cf = self.load(&cockpit)?;
        let him = cf.strings().find(|s| {
            let l = s.to_ascii_lowercase();
            l.starts_with("3do/cockpit/") && l.ends_with(".him")
        })?;
        Some((cockpit, him.to_owned()))
    }

    /// The classes a cockpit's instrument code runs on: the cockpit class and its superclasses,
    /// their inner classes, and the few world helpers it computes with (`anim::Instruments`).
    pub fn cockpit_pool(&self, cockpit: &str) -> Vec<classfile::ClassFile> {
        let mut pool = self.chain(cockpit);
        for helper in [
            "com.maddox.il2.ai.AnglesFork",
            "com.maddox.il2.fm.Pitot",
            "com.maddox.il2.fm.Atmosphere",
        ] {
            pool.extend(self.chain(helper).into_iter().take(1));
        }
        let mut i = 0;
        while i < pool.len() && pool.len() < 96 {
            let outer = pool[i]
                .this_class
                .split('$')
                .next()
                .unwrap_or("")
                .to_owned();
            let inner: Vec<String> = pool[i]
                .consts
                .iter()
                .filter_map(|c| match c {
                    classfile::Const::Class(n) => pool[i].utf8(*n).map(str::to_owned),
                    _ => None,
                })
                .filter(|n| n.starts_with(&format!("{outer}$")))
                .collect();
            for n in inner {
                if !pool.iter().any(|c| c.this_class == n) {
                    if let Some(c) = self.load(&n.replace('/', ".")) {
                        pool.push(c);
                    }
                }
            }
            i += 1;
        }
        pool
    }

    /// Evaluates a weapon class's `createProperties()` (gun and bullet data), e.g. `MGunMG131si`.
    pub fn gun_properties(&self, weapon: &str) -> Result<anim::Value, String> {
        let full = format!("com.maddox.il2.objects.weapons.{weapon}");
        let pool = self.pool(&full);
        if pool.is_empty() {
            return Err(format!("{weapon}: class not found"));
        }
        anim::evaluate(
            &pool,
            "createProperties",
            "()Lcom/maddox/il2/engine/GunProperties;",
        )
    }

    /// The string constants of a class (e.g. model paths named in its code).
    pub fn class_strings(&self, class: &str) -> Vec<String> {
        let full = if class.starts_with("com.") {
            class.to_owned()
        } else {
            format!("com.maddox.il2.objects.{class}")
        };
        self.load(&full)
            .map(|c| c.strings().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// Short names of the interfaces the class and its superclasses implement (`TypeFighter`, …).
    pub fn interfaces(&self, class: &str) -> Vec<String> {
        let full = if class.starts_with("com.") {
            class.to_owned()
        } else {
            format!("com.maddox.il2.objects.{class}")
        };
        self.chain(&full)
            .iter()
            .flat_map(|c| {
                c.interfaces
                    .iter()
                    .map(|i| i.rsplit('/').next().unwrap_or("").to_owned())
            })
            .collect()
    }

    /// Short names of the class and its superclasses, most derived first (`BF_109G6`, `BF_109`, …).
    pub fn lineage(&self, class: &str) -> Vec<String> {
        let full = if class.starts_with("com.") {
            class.to_owned()
        } else {
            format!("com.maddox.il2.objects.{class}")
        };
        self.chain(&full)
            .iter()
            .map(|c| c.this_class.rsplit('/').next().unwrap_or("").to_owned())
            .collect()
    }

    /// For each channel, whether its animation was imported, else why not (diagnostics).
    pub fn animation_report(&self, class: &str) -> Vec<(anim::Channel, Result<(), String>)> {
        let full = if class.starts_with("com.") {
            class.to_owned()
        } else {
            format!("com.maddox.il2.objects.{class}")
        };
        let chain = self.chain(&full);
        anim::Channel::ALL
            .iter()
            .map(|&ch| {
                (
                    ch,
                    anim::sample(&chain, ch)
                        .map(|_| ())
                        .ok_or_else(anim::last_abort),
                )
            })
            .collect()
    }

    /// Part animation tracks of an aircraft class, imported where possible, else the generic defaults.
    pub fn animations(&self, class: &str) -> Vec<anim::Track> {
        let full = if class.starts_with("com.") {
            class.to_owned()
        } else {
            format!("com.maddox.il2.objects.{class}")
        };
        let chain = self.chain(&full);
        anim::Channel::ALL
            .iter()
            .map(|&ch| anim::sample(&chain, ch).unwrap_or_else(|| anim::default_track(ch)))
            .collect()
    }

    /// `air.BF_109G6` or `com.maddox.il2.objects.air.BF_109G6`.
    pub fn aircraft(&self, class: &str) -> Option<AircraftClass> {
        let full = if class.starts_with("com.") {
            class.to_owned()
        } else {
            format!("com.maddox.il2.objects.{class}")
        };
        let c = self.load(&full)?;
        let mut flight_model = None;
        let mut meshes: Vec<String> = Vec::new();
        let mut default = None;
        let mut last = "";
        for s in c.strings() {
            let l = s.to_ascii_lowercase();
            if l.starts_with("flightmodels/") && l.ends_with(".fmd") && flight_model.is_none() {
                flight_model = Some(s.to_owned());
            } else if l.starts_with("3do/plane/")
                && l.ends_with(".him")
                && !meshes.iter().any(|m| m == s)
            {
                // `Property.set(cls, "meshName", path)`: the key directly precedes its value in the
                // constant pool.
                if last == "meshName" && default.is_none() {
                    default = Some(s.to_owned());
                }
                meshes.push(s.to_owned());
            }
            last = s;
        }
        meshes.sort_by_key(|m| (Some(m) != default.as_ref(), m.contains('(')));
        Some(AircraftClass {
            flight_model: flight_model?,
            meshes,
        })
    }
}

/// Access to `FlightModels/*.fmd` and `*.emd` section files.
pub struct FlightModels {
    pub keys: KeyTable,
    container: container::Container,
}

impl FlightModels {
    pub fn open(vfs: &Vfs) -> Result<Self, Error> {
        let keys = KeyTable::from_install(vfs)?;
        let container = container::Container::parse(vfs.read("gui/game/buttons")?)?;
        Ok(FlightModels { keys, container })
    }

    pub fn len(&self) -> usize {
        self.container.len()
    }

    pub fn is_empty(&self) -> bool {
        self.container.is_empty()
    }

    /// Raw decoded text of e.g. `FlightModels/Bf-109G-6Early.fmd`.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, Error> {
        let l = name.to_lowercase();
        let mut data = self
            .container
            .read(&int_hash(&format!("{l}d2w0")).to_string())?
            .ok_or_else(|| Error::NotFound(name.to_owned()))?;
        let key = self.keys.fm_key(int_hash(&format!("{l}ogh9")), data.len());
        keys::unscramble_stream(&mut data, &key);
        Ok(data)
    }

    pub fn sect_file(&self, name: &str) -> Result<SectFile, Error> {
        Ok(SectFile::parse(&self.read(name)?))
    }
}
