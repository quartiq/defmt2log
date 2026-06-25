use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use crate::Info;
use defmt_decoder::{DecodeOptions, Table};
use findshlibs::{IterationControl, SharedLibrary, TargetSharedLibrary};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(crate) fn load_host_state(elf: &[u8], path: &Path) -> Result<Info> {
    // Host frames contain loaded PIE addresses truncated to u16. The decoder
    // table uses linked symbol VMAs, so normalize frame indices at the stream
    // boundary before decoding.
    let load_bias = mapped_executable_slide(path)?;
    let table =
        Table::parse(elf)?.ok_or("current executable does not contain any defmt metadata")?;
    build_state(elf, table, load_bias)
}

pub(crate) fn load_merged_state(elf: &[u8]) -> Result<Info> {
    let table = Table::parse(elf)?.ok_or("ELF has no merged `.defmt` section")?;
    build_state(elf, table, 0)
}

fn build_state(elf: &[u8], table: Table, address_bias: u64) -> Result<Info> {
    let locations = table.get_locations(elf).unwrap_or_else(|err| {
        log::warn!("defmt2log: failed to load source locations: {err}");
        Default::default()
    });
    let decode_index = table.new_decode_index(DecodeOptions::new().address_bias(address_bias));
    Ok(Info {
        table,
        decode_index,
        locations,
    })
}

fn mapped_executable_slide(path: &Path) -> Result<u64> {
    let expected = canonicalize_lenient(path);
    let mut slide = None;
    // `findshlibs` is backed by the platform loader (`dl_iterate_phdr` on
    // Linux). The returned virtual-memory bias is the value to add to ELF
    // symbol VMAs for a PIE executable.
    TargetSharedLibrary::each(|library| {
        if mapped_path_matches(Path::new(library.name()), &expected) {
            slide = Some(library.virtual_memory_bias().0 as u64);
            IterationControl::Break
        } else {
            IterationControl::Continue
        }
    });

    slide.ok_or_else(|| {
        format!(
            "failed to find loader mapping for {} via findshlibs",
            expected.display()
        )
        .into()
    })
}

fn canonicalize_lenient(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn mapped_path_matches(mapped_path: &Path, expected: &Path) -> bool {
    // Linux can report deleted/replaced executables with a " (deleted)"
    // suffix. Canonicalize both forms so current_exe() still matches the
    // loader's path spelling.
    canonicalize_lenient(mapped_path) == expected
        || mapped_path
            .to_str()
            .and_then(|path| path.strip_suffix(" (deleted)"))
            .is_some_and(|path| canonicalize_lenient(Path::new(path)) == expected)
}
