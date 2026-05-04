#![doc = include_str!("../README.md")]

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use defmt_decoder::{Locations, Table};

mod logger;
mod table;

#[derive(Debug)]
pub enum InitError {
    AlreadyInitialized,
    CurrentExe(std::io::Error),
    ReadElf {
        path: PathBuf,
        source: std::io::Error,
    },
    ParseTable(String),
    MissingDefmtSection,
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyInitialized => {
                write!(f, "defmt2log is already initialized")
            }
            Self::CurrentExe(err) => {
                write!(f, "failed to locate current executable: {err}")
            }
            Self::ReadElf { path, source } => {
                write!(f, "failed to read ELF {}: {source}", path.display())
            }
            Self::ParseTable(err) => {
                write!(f, "failed to parse defmt table: {err}")
            }
            Self::MissingDefmtSection => {
                write!(f, "current executable does not contain any defmt metadata")
            }
        }
    }
}

impl std::error::Error for InitError {}

pub(crate) struct State {
    pub(crate) table: Table,
    pub(crate) locations: Option<Locations>,
}

static STATE: OnceLock<State> = OnceLock::new();

/// Initialize from the current host executable.
pub fn init_from_current_exe() -> Result<(), InitError> {
    let path = env::current_exe().map_err(InitError::CurrentExe)?;
    init_from_elf_path_with_fallback(path, true)
}

/// Initialize from an explicit ELF path.
///
/// This always supports direct parsing of an ELF that already contains a
/// merged `.defmt` section. The synthetic host fallback is only used when the
/// path is the current executable.
pub fn init_from_elf_path(path: impl AsRef<Path>) -> Result<(), InitError> {
    let path = path.as_ref();
    init_from_elf_path_with_fallback(path, is_current_executable(path))
}

fn init_from_elf_path_with_fallback(
    path: impl AsRef<Path>,
    allow_host_fallback: bool,
) -> Result<(), InitError> {
    let path = path.as_ref();
    let elf = fs::read(path).map_err(|source| InitError::ReadElf {
        path: path.to_path_buf(),
        source,
    })?;
    let fallback_path = allow_host_fallback.then_some(path);
    init_state(table::load_state(&elf, fallback_path)?)
}

/// Initialize from explicit ELF bytes that already contain a merged `.defmt`
/// section.
///
/// This does not perform the host synthetic fallback. For normal host
/// executables, use [`init_from_current_exe`].
pub fn init_from_merged_elf_bytes(elf: &[u8]) -> Result<(), InitError> {
    init_state(table::load_state(elf, None)?)
}

fn is_current_executable(path: &Path) -> bool {
    let Ok(current) = env::current_exe() else {
        return false;
    };
    let current = fs::canonicalize(&current).unwrap_or(current);
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path == current
}

fn init_state(state: State) -> Result<(), InitError> {
    STATE.set(state).map_err(|_| InitError::AlreadyInitialized)
}

pub(crate) fn state() -> &'static State {
    STATE
        .get()
        .expect("defmt2log must be initialized before emitting defmt logs")
}

#[cfg(test)]
mod test {
    #[test]
    fn smoke() {
        env_logger::init();
        crate::init_from_current_exe().unwrap();
        defmt::info!("word {=u32:#010x}", 0x1234u32);
    }
}
