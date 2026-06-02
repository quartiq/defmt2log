use std::{
    collections::HashMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use crate::Info;
use defmt_decoder::{Locations, Table, Tag};
use findshlibs::{IterationControl, SharedLibrary, TargetSharedLibrary};
use object::{
    BinaryFormat, Object, ObjectKind, ObjectSection, ObjectSymbol, SectionIndex, SectionKind,
    SymbolFlags, SymbolKind, SymbolScope,
    write::{Object as WriteObject, Symbol, SymbolSection},
};
use serde_json::Value;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct ParsedTable {
    table: Table,
    symbol_indices: Option<HashMap<String, u64>>,
}

struct DefmtSymbol {
    name: String,
    runtime_index: u16,
    size: u64,
    data: Vec<u8>,
}

struct SyntheticInput<'a> {
    object: object::File<'a>,
    path: &'a Path,
}

struct Metadata {
    version: Option<String>,
    encoding: Option<String>,
}

pub(crate) fn load_host_state(elf: &[u8], path: &Path) -> Result<Info> {
    if has_merged_defmt_section(elf)? {
        return load_merged_state(elf);
    }

    let parsed = SyntheticInput::parse(elf, path)?
        .build_table()?
        .ok_or("current executable does not contain any defmt metadata")?;
    build_state(elf, parsed)
}

pub(crate) fn load_merged_state(elf: &[u8]) -> Result<Info> {
    let table = Table::parse(elf)?.ok_or("ELF has no merged `.defmt` section")?;
    build_state(
        elf,
        ParsedTable {
            table,
            symbol_indices: None,
        },
    )
}

fn has_merged_defmt_section(elf: &[u8]) -> Result<bool> {
    let object = object::File::parse(elf).map_err(|err| err.to_string())?;
    Ok(object.section_by_name(".defmt").is_some())
}

fn build_state(elf: &[u8], parsed: ParsedTable) -> Result<Info> {
    let locations = load_locations(elf, &parsed).unwrap_or_else(|err| {
        log::warn!("defmt2log: failed to load source locations: {err}");
        Default::default()
    });
    Ok(Info {
        table: parsed.table,
        locations,
    })
}

fn load_locations(elf: &[u8], parsed: &ParsedTable) -> Result<Locations> {
    let raw_locations = parsed
        .table
        .get_locations(elf)
        .map_err(|err| err.to_string())?;

    if let Some(symbol_indices) = &parsed.symbol_indices {
        let object = object::File::parse(elf).map_err(|err| err.to_string())?;
        let original_addresses = original_symbol_addresses(&object);
        let original_locations = original_locations_by_symbol(&raw_locations, &original_addresses);
        let remapped = symbol_indices
            .iter()
            .filter_map(|(name, index)| {
                original_locations
                    .get(name)
                    .cloned()
                    .map(|location| (*index, location))
            })
            .collect();
        Ok(remapped)
    } else {
        Ok(raw_locations)
    }
}

fn build_synthetic_table(
    object: &object::File<'_>,
    symbols: &[DefmtSymbol],
    encoding: &str,
    version: &str,
) -> Result<ParsedTable> {
    let mut synthetic = WriteObject::new(
        BinaryFormat::Elf,
        object.architecture(),
        object.endianness(),
    );
    synthetic.add_file_symbol(b"defmt2log-synthetic".to_vec());
    let defmt = synthetic.add_section(Vec::new(), b".defmt".to_vec(), SectionKind::ReadOnlyData);

    let max_size = symbols
        .iter()
        .map(|symbol| u64::from(symbol.runtime_index) + symbol.size)
        .max()
        .unwrap_or(0);
    synthetic.set_section_data(defmt, vec![0; max_size as usize], 1);

    let mut symbol_indices = HashMap::new();
    for symbol in symbols {
        if symbol_indices
            .insert(symbol.name.clone(), u64::from(symbol.runtime_index))
            .is_some()
        {
            return Err(format!("duplicate synthetic defmt symbol name {}", symbol.name).into());
        }

        synthetic.add_symbol(Symbol {
            name: symbol.name.as_bytes().to_vec(),
            value: u64::from(symbol.runtime_index),
            size: symbol.size,
            kind: SymbolKind::Data,
            scope: SymbolScope::Linkage,
            weak: false,
            section: SymbolSection::Section(defmt),
            flags: SymbolFlags::None,
        });

        let start = usize::from(symbol.runtime_index);
        let end = start + symbol.data.len();
        synthetic.section_mut(defmt).data_mut()[start..end].copy_from_slice(&symbol.data);
    }

    add_metadata_symbol(&mut synthetic, format!("_defmt_encoding_ = {encoding}"));
    add_metadata_symbol(&mut synthetic, format!("_defmt_version_ = {version}"));

    let elf = synthetic.write().map_err(|err| err.to_string())?;
    let table = Table::parse(&elf)
        .map_err(|err| err.to_string())?
        .ok_or("synthetic ELF lost `.defmt`")?;

    Ok(ParsedTable {
        table,
        symbol_indices: Some(symbol_indices),
    })
}

impl<'a> SyntheticInput<'a> {
    fn parse(elf: &'a [u8], path: &'a Path) -> Result<Self> {
        Ok(Self {
            object: object::File::parse(elf).map_err(|err| err.to_string())?,
            path,
        })
    }

    fn build_table(&self) -> Result<Option<ParsedTable>> {
        let sections = self.defmt_sections();
        let metadata = self.metadata()?;

        match (sections.is_empty(), &metadata.version, &metadata.encoding) {
            (true, None, None) => return Ok(None),
            (true, _, _) => {
                return Err("defmt metadata found, but no `.defmt.*` sections were found".into());
            }
            (false, None, _) => {
                return Err("found `.defmt.*` sections, but no defmt version symbol".into());
            }
            (false, _, None) => {
                return Err("found `.defmt.*` sections, but no defmt encoding symbol".into());
            }
            (false, Some(version), Some(_))
                if !defmt_decoder::DEFMT_VERSIONS.contains(&version.as_str()) =>
            {
                return Err(format!("unsupported defmt version {version}").into());
            }
            (false, Some(_), Some(_)) => {}
        }

        let symbols = self.collect_symbols(&sections, self.runtime_slide()?)?;
        build_synthetic_table(
            &self.object,
            &symbols,
            metadata.encoding.as_deref().unwrap(),
            metadata.version.as_deref().unwrap(),
        )
        .map(Some)
    }

    fn defmt_sections(&self) -> Vec<SectionIndex> {
        self.object
            .sections()
            .filter_map(|section| {
                section
                    .name()
                    .ok()
                    .filter(|name| is_defmt_section(name))
                    .map(|_| section.index())
            })
            .collect()
    }

    fn metadata(&self) -> Result<Metadata> {
        let mut version = None;
        let mut encoding = None;

        for symbol in self.object.symbols() {
            let Ok(name) = symbol.name() else {
                continue;
            };

            if let Some(new_version) = parse_version(name)
                && let Some(old_version) = version.replace(new_version.clone())
            {
                return Err(format!(
                    "multiple defmt versions in use: {} and {} (only one is supported)",
                    old_version, new_version
                )
                .into());
            }

            if let Some(new_encoding) = parse_encoding(name)
                && let Some(old_encoding) = encoding.replace(new_encoding.clone())
            {
                return Err(format!(
                    "multiple defmt encodings in use: {} and {} (only one is supported)",
                    old_encoding, new_encoding
                )
                .into());
            }
        }

        Ok(Metadata { version, encoding })
    }

    fn runtime_slide(&self) -> Result<u64> {
        if self.object.kind() != ObjectKind::Dynamic {
            return Ok(0);
        }

        mapped_executable_slide(self.path)
    }

    fn collect_symbols(&self, sections: &[SectionIndex], slide: u64) -> Result<Vec<DefmtSymbol>> {
        let mut symbols = Vec::new();
        let mut seen_indices = HashMap::new();
        for symbol in self.object.symbols() {
            let Ok(name) = symbol.name() else {
                continue;
            };
            if skip_symbol(name) {
                continue;
            }
            let Some(section_index) = symbol.section_index() else {
                continue;
            };
            if !sections.contains(&section_index) {
                continue;
            }

            let Some(tag) = symbol_tag(name)? else {
                continue;
            };

            let runtime_index = symbol.address().wrapping_add(slide) as u16;
            if let Some(old) = seen_indices.insert(runtime_index, name.to_owned()) {
                return Err(format!(
                    "runtime defmt index collision {runtime_index:#06x}: {old} and {name}"
                )
                .into());
            }

            let size = symbol.size().max(1);
            let data = if tag == Tag::BitflagsValue {
                symbol_bytes(&self.object, name, section_index, symbol.address(), size)?
            } else {
                vec![0; size as usize]
            };

            symbols.push(DefmtSymbol {
                name: name.to_owned(),
                runtime_index,
                size,
                data,
            });
        }
        Ok(symbols)
    }
}

fn add_metadata_symbol(object: &mut WriteObject<'_>, name: String) {
    object.add_symbol(Symbol {
        name: name.into_bytes(),
        value: 0,
        size: 0,
        kind: SymbolKind::Data,
        scope: SymbolScope::Compilation,
        weak: false,
        section: SymbolSection::Absolute,
        flags: SymbolFlags::None,
    });
}

fn symbol_bytes(
    object: &object::File<'_>,
    name: &str,
    section_index: SectionIndex,
    address: u64,
    size: u64,
) -> Result<Vec<u8>> {
    let section = object
        .section_by_index(section_index)
        .map_err(|err| err.to_string())?;
    let data = section
        .data_range(address, size)
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("defmt symbol `{name}` lies outside its section"))?;
    Ok(data.to_vec())
}

fn mapped_executable_slide(path: &Path) -> Result<u64> {
    let expected = canonicalize_lenient(path);
    let mut slide = None;
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
    canonicalize_lenient(mapped_path) == expected
        || mapped_path
            .to_str()
            .and_then(|path| path.strip_suffix(" (deleted)"))
            .is_some_and(|path| canonicalize_lenient(Path::new(path)) == expected)
}

fn original_symbol_addresses(object: &object::File<'_>) -> HashMap<String, u64> {
    object
        .symbols()
        .filter_map(|symbol| {
            let name = symbol.name().ok()?;
            if skip_symbol(name) {
                return None;
            }
            Some((name.to_owned(), symbol.address()))
        })
        .collect()
}

fn original_locations_by_symbol(
    locations: &Locations,
    addresses: &HashMap<String, u64>,
) -> HashMap<String, defmt_decoder::Location> {
    addresses
        .iter()
        .filter_map(|(name, address)| {
            locations
                .get(address)
                .cloned()
                .map(|location| (name.clone(), location))
        })
        .collect()
}

fn is_defmt_section(name: &str) -> bool {
    name == ".defmt" || name.starts_with(".defmt.") || name.starts_with(".defmt,")
}

fn parse_version(name: &str) -> Option<String> {
    name.strip_prefix("\"_defmt_version_ = ")
        .or_else(|| name.strip_prefix("_defmt_version_ = "))
        .map(|version| version.trim_end_matches('"').to_owned())
}

fn parse_encoding(name: &str) -> Option<String> {
    name.strip_prefix("_defmt_encoding_ = ")
        .map(ToOwned::to_owned)
}

fn skip_symbol(name: &str) -> bool {
    name.is_empty()
        || name == "$d"
        || name.starts_with("$d.")
        || name.starts_with("_defmt")
        || name.starts_with("__DEFMT_MARKER")
}

fn symbol_tag(raw: &str) -> Result<Option<Tag>> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|err| format!("failed to parse defmt symbol `{raw}`: {err}"))?;
    let tag = value
        .get("tag")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("defmt symbol missing string `tag`: `{raw}`"))?;

    Ok(match tag {
        "defmt_prim" => Some(Tag::Prim),
        "defmt_derived" => Some(Tag::Derived),
        "defmt_bitflags" => Some(Tag::Bitflags),
        "defmt_write" => Some(Tag::Write),
        "defmt_timestamp" => Some(Tag::Timestamp),
        "defmt_bitflags_value" => Some(Tag::BitflagsValue),
        "defmt_str" => Some(Tag::Str),
        "defmt_println" => Some(Tag::Println),
        "defmt_trace" => Some(Tag::Trace),
        "defmt_debug" => Some(Tag::Debug),
        "defmt_info" => Some(Tag::Info),
        "defmt_warn" => Some(Tag::Warn),
        "defmt_error" => Some(Tag::Error),
        _ => None,
    })
}
