#![doc = include_str!("../README.md")]

use std::{env, fs, path::Path, sync::OnceLock};

use defmt_decoder::{Locations, Table};

mod logger;
mod table;

pub(crate) struct State {
    pub(crate) table: Table,
    pub(crate) locations: Option<Locations>,
}

static STATE: OnceLock<State> = OnceLock::new();

/// Initialize from the current host executable.
///
/// Panics if the current executable cannot be located, read, decoded, or if
/// defmt2log was already initialized.
pub fn init_from_current_exe() {
    let path = env::current_exe()
        .unwrap_or_else(|err| panic!("defmt2log failed to locate current executable: {err}"));
    let elf = read_elf(&path);
    init_state(table::load_host_state(&elf, &path));
}

/// Initialize from an explicit ELF path that already contains a merged
/// `.defmt` section.
///
/// Panics if the file cannot be read, does not contain a merged `.defmt`
/// section, cannot be decoded, or if defmt2log was already initialized.
pub fn init_from_merged_elf_path(path: impl AsRef<Path>) {
    let path = path.as_ref();
    let elf = read_elf(path);
    init_state(table::load_merged_state(&elf));
}

/// Initialize from explicit ELF bytes that already contain a merged `.defmt`
/// section.
///
/// Panics if the bytes do not contain a merged `.defmt` section, cannot be
/// decoded, or if defmt2log was already initialized.
pub fn init_from_merged_elf_bytes(elf: &[u8]) {
    init_state(table::load_merged_state(elf));
}

fn read_elf(path: &Path) -> Vec<u8> {
    fs::read(path)
        .unwrap_or_else(|err| panic!("defmt2log failed to read ELF {}: {err}", path.display()))
}

fn init_state(state: Result<State, String>) {
    let state = state.unwrap_or_else(|err| panic!("defmt2log initialization failed: {err}"));
    STATE
        .set(state)
        .unwrap_or_else(|_| panic!("defmt2log is already initialized"));
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
        crate::init_from_current_exe();
        defmt::info!("word {=u32:#010x}", 0x1234u32);
    }
}
