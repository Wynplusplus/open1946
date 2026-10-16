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

    /// Short names of the class and its superclasses, most derived first (`BF_109G6`, `BF_109`, …).
    pub fn lineage(&self, class: &str) -> Vec<String> {
        let full = if class.starts_with("com.") { class.to_owned() } else { format!("com.maddox.il2.objects.{class}") };
        self.chain(&full).iter().map(|c| c.this_class.rsplit('/').next().unwrap_or("").to_owned()).collect()
    }

    /// For each channel, whether its animation was imported, else why not (diagnostics).
    pub fn animation_report(&self, class: &str) -> Vec<(anim::Channel, Result<(), String>)> {
        let full = if class.starts_with("com.") { class.to_owned() } else { format!("com.maddox.il2.objects.{class}") };
        let chain = self.chain(&full);
        anim::Channel::ALL
            .iter()
            .map(|&ch| (ch, anim::sample(&chain, ch).map(|_| ()).ok_or_else(anim::last_abort)))
            .collect()
    }

    /// Part animation tracks of an aircraft class, imported where possible, else the generic defaults.
    pub fn animations(&self, class: &str) -> Vec<anim::Track> {
        let full = if class.starts_with("com.") { class.to_owned() } else { format!("com.maddox.il2.objects.{class}") };
        let chain = self.chain(&full);
        anim::Channel::ALL
            .iter()
            .map(|&ch| anim::sample(&chain, ch).unwrap_or_else(|| anim::default_track(ch)))
            .collect()
    }

    /// `air.BF_109G6` or `com.maddox.il2.objects.air.BF_109G6`.
    pub fn aircraft(&self, class: &str) -> Option<AircraftClass> {
        let full = if class.starts_with("com.") { class.to_owned() } else { format!("com.maddox.il2.objects.{class}") };
        let c = self.load(&full)?;
        let mut flight_model = None;
        let mut meshes: Vec<String> = Vec::new();
        let mut default = None;
        let mut last = "";
        for s in c.strings() {
            let l = s.to_ascii_lowercase();
            if l.starts_with("flightmodels/") && l.ends_with(".fmd") && flight_model.is_none() {
                flight_model = Some(s.to_owned());
            } else if l.starts_with("3do/plane/") && l.ends_with(".him") && !meshes.iter().any(|m| m == s) {
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
        Some(AircraftClass { flight_model: flight_model?, meshes })
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
