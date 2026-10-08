#![doc = include_str!("../README.md")]

use std::{env, error::Error, fs, sync::LazyLock};

use defmt_decoder::{DecodeContext, Locations, Table};

mod logger;

pub(crate) struct State {
    pub(crate) table: Table,
    pub(crate) decode_context: DecodeContext,
    pub(crate) locations: Locations,
}

pub(crate) static STATE: LazyLock<State> =
    LazyLock::new(|| load_current_exe().expect("defmt2log initialization failed"));

/// Initialize from the current host executable.
///
/// Calling this is optional: the first defmt log frame initializes lazily too.
/// Call it when initialization failures should happen before the first log.
pub fn init() {
    LazyLock::force(&STATE);
}

fn load_current_exe() -> Result<State, Box<dyn Error>> {
    let path = env::current_exe()?;
    let elf = fs::read(path)?;
    let table =
        Table::parse(&elf)?.ok_or("current executable does not contain any defmt metadata")?;
    let decode_context = table
        .new_decode_context_for_runtime_anchor(defmt::runtime_anchor() as u64)
        .ok_or("current executable defmt metadata does not contain a runtime anchor")?;
    let locations = table.get_locations(&elf).unwrap_or_else(|err| {
        log::warn!("defmt2log: failed to load source locations: {err}");
        Default::default()
    });
    Ok(State {
        table,
        decode_context,
        locations,
    })
}
