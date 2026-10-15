//! Game data that the runtime keeps scrambled: flight models and engine definitions.
//! Formats: `docs/formats/krypto.md`. All key material is read from the user's install.

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

/// Access to `FlightModels/*.fmd` and `*.emd` section files.
pub struct FlightModels {
    keys: KeyTable,
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
