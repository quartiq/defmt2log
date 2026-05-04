use std::{collections::HashMap, fs, path::Path, sync::Once};

use crate::{InitError, State};
use defmt_decoder::{Locations, StringEntry, Table, TableEntry, Tag};
use object::{
    BinaryFormat, Object, ObjectKind, ObjectSection, ObjectSegment,
    ObjectSymbol, SectionIndex, SectionKind, SymbolFlags, SymbolKind,
    SymbolScope,
    write::{Object as WriteObject, Symbol, SymbolSection},
};

static WARNED_LOCATIONS: Once = Once::new();

struct ParsedTable {
    table: Table,
    symbol_indices: Option<HashMap<String, usize>>,
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

pub(crate) fn load_state(
    elf: &[u8],
    path: Option<&Path>,
) -> Result<State, InitError> {
    let parsed =
        parse_table(elf, path)?.ok_or(InitError::MissingDefmtSection)?;
    let locations = match load_locations(elf, &parsed) {
        Ok(locations) => Some(locations),
        Err(err) => {
            WARNED_LOCATIONS.call_once(|| {
                log::warn!("defmt2log: failed to load source locations: {err}");
            });
            None
        }
    };
    Ok(State {
        table: parsed.table,
        locations,
    })
}

fn parse_table(
    elf: &[u8],
    path: Option<&Path>,
) -> Result<Option<ParsedTable>, InitError> {
    if let Some(table) = parse_merged_table(elf)? {
        return Ok(Some(ParsedTable {
            table,
            symbol_indices: None,
        }));
    }

    let Some(path) = path else {
        return Err(InitError::ParseTable(
            "ELF has no merged `.defmt` section; synthetic host fallback is only available for the current executable via init_from_current_exe() or init_from_elf_path(current_exe)"
                .to_owned(),
        ));
    };

    SyntheticInput::parse(elf, path)?.build_table()
}

fn load_locations(
    elf: &[u8],
    parsed: &ParsedTable,
) -> Result<Locations, String> {
    let raw_locations = parsed
        .table
        .get_locations(elf)
        .map_err(|err| err.to_string())?;

    if let Some(symbol_indices) = &parsed.symbol_indices {
        let object = object::File::parse(elf).map_err(|err| err.to_string())?;
        let original_addresses = original_symbol_addresses(&object);
        let original_locations =
            original_locations_by_symbol(&raw_locations, &original_addresses);
        let remapped = symbol_indices
            .iter()
            .filter_map(|(name, index)| {
                original_locations
                    .get(name)
                    .cloned()
                    .map(|location| (*index as u64, location))
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
) -> Result<Option<ParsedTable>, InitError> {
    let mut synthetic = WriteObject::new(
        BinaryFormat::Elf,
        object.architecture(),
        object.endianness(),
    );
    synthetic.add_file_symbol(b"defmt2log-synthetic".to_vec());
    let defmt = synthetic.add_section(
        Vec::new(),
        b".defmt".to_vec(),
        SectionKind::ReadOnlyData,
    );

    let max_size = symbols
        .iter()
        .map(|symbol| u64::from(symbol.runtime_index) + symbol.size)
        .max()
        .unwrap_or(0);
    synthetic.set_section_data(defmt, vec![0; max_size as usize], 1);

    let mut symbol_indices = HashMap::new();
    for symbol in symbols {
        if symbol_indices
            .insert(symbol.name.clone(), usize::from(symbol.runtime_index))
            .is_some()
        {
            return Err(InitError::ParseTable(format!(
                "duplicate synthetic defmt symbol name {}",
                symbol.name
            )));
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
        synthetic.section_mut(defmt).data_mut()[start..end]
            .copy_from_slice(&symbol.data);
    }

    add_metadata_symbol(
        &mut synthetic,
        format!("_defmt_encoding_ = {encoding}"),
    );
    add_metadata_symbol(&mut synthetic, format!("_defmt_version_ = {version}"));

    let elf = synthetic
        .write()
        .map_err(|err| InitError::ParseTable(err.to_string()))?;
    let mut table = Table::parse(&elf)
        .map_err(|err| InitError::ParseTable(err.to_string()))?
        .ok_or_else(|| {
            InitError::ParseTable("synthetic ELF lost `.defmt`".to_owned())
        })?;
    if !table.has_timestamp() {
        table.set_timestamp_entry(host_timestamp_entry());
    }

    Ok(Some(ParsedTable {
        table,
        symbol_indices: Some(symbol_indices),
    }))
}

impl<'a> SyntheticInput<'a> {
    fn parse(elf: &'a [u8], path: &'a Path) -> Result<Self, InitError> {
        Ok(Self {
            object: object::File::parse(elf)
                .map_err(|err| InitError::ParseTable(err.to_string()))?,
            path,
        })
    }

    fn build_table(&self) -> Result<Option<ParsedTable>, InitError> {
        let sections = self.defmt_sections();
        let version = self.version();
        let encoding = self.encoding();

        match (sections.is_empty(), version.as_deref()) {
            (true, None) => return Ok(None),
            (true, Some(_)) => {
                return Err(InitError::ParseTable(
                    "defmt version found, but no `.defmt.*` sections were found"
                        .to_owned(),
                ));
            }
            (false, None) => {
                return Err(InitError::ParseTable(
                    "found `.defmt.*` sections, but no defmt version symbol"
                        .to_owned(),
                ));
            }
            (false, Some(version))
                if !defmt_decoder::DEFMT_VERSIONS.contains(&version) =>
            {
                return Err(InitError::ParseTable(format!(
                    "unsupported defmt version {version}"
                )));
            }
            (false, Some(_)) => {}
        }

        let encoding = encoding.ok_or_else(|| {
            InitError::ParseTable("no defmt encoding symbol found".to_owned())
        })?;
        let version = version.expect("validated above");
        let symbols = self.collect_symbols(&sections, self.runtime_slide()?)?;
        if symbols.is_empty() {
            return Ok(None);
        }

        build_synthetic_table(&self.object, &symbols, &encoding, &version)
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

    fn version(&self) -> Option<String> {
        self.object
            .symbols()
            .find_map(|symbol| symbol.name().ok().and_then(parse_version))
    }

    fn encoding(&self) -> Option<String> {
        self.object
            .symbols()
            .find_map(|symbol| symbol.name().ok().and_then(parse_encoding))
    }

    fn runtime_slide(&self) -> Result<u64, InitError> {
        if self.object.kind() != ObjectKind::Dynamic {
            return Ok(0);
        }

        let base_segment = self
            .object
            .segments()
            .filter(|segment| segment.file_range().0 == 0)
            .min_by_key(|segment| segment.address())
            .ok_or_else(|| {
                InitError::ParseTable(
                    "no loadable file-offset-zero segment in ELF".to_owned(),
                )
            })?;
        let runtime_base = mapped_executable_base(self.path)?;
        Ok(runtime_base.wrapping_sub(base_segment.address()))
    }

    fn collect_symbols(
        &self,
        sections: &[SectionIndex],
        slide: u64,
    ) -> Result<Vec<DefmtSymbol>, InitError> {
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
            if let Some(old) =
                seen_indices.insert(runtime_index, name.to_owned())
            {
                return Err(InitError::ParseTable(format!(
                    "runtime defmt index collision {runtime_index:#06x}: {old} and {name}"
                )));
            }

            let size = symbol.size().max(1);
            let data = if tag == Tag::BitflagsValue {
                symbol_bytes(
                    &self.object,
                    name,
                    section_index,
                    symbol.address(),
                    size,
                )?
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
) -> Result<Vec<u8>, InitError> {
    let section = object
        .section_by_index(section_index)
        .map_err(|err| InitError::ParseTable(err.to_string()))?;
    let data = section
        .data_range(address, size)
        .map_err(|err| InitError::ParseTable(err.to_string()))?
        .ok_or_else(|| {
            InitError::ParseTable(format!(
                "defmt symbol `{}` lies outside its section",
                name
            ))
        })?;
    Ok(data.to_vec())
}

fn mapped_executable_base(path: &Path) -> Result<u64, InitError> {
    let expected =
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let maps = fs::read_to_string("/proc/self/maps").map_err(|err| {
        InitError::ParseTable(format!("failed to read /proc/self/maps: {err}"))
    })?;
    let mut best: Option<u64> = None;
    for line in maps.lines() {
        let mut parts = line.split_whitespace();
        let Some(range) = parts.next() else {
            continue;
        };
        let _perms = parts.next();
        let Some(offset) = parts.next() else {
            continue;
        };
        let _dev = parts.next();
        let _inode = parts.next();
        let Some(mapped_path) = parts.next() else {
            continue;
        };
        if offset != "00000000" {
            continue;
        }
        let mapped = Path::new(mapped_path);
        let mapped =
            fs::canonicalize(mapped).unwrap_or_else(|_| mapped.to_path_buf());
        if mapped != expected {
            continue;
        }
        let Some((start, _end)) = range.split_once('-') else {
            continue;
        };
        let start = u64::from_str_radix(start, 16).map_err(|err| {
            InitError::ParseTable(format!(
                "failed to parse `/proc/self/maps` address: {err}"
            ))
        })?;
        best = Some(best.map_or(start, |old| old.min(start)));
    }

    best.ok_or_else(|| {
        InitError::ParseTable(format!(
            "failed to find offset-zero mapping for {} in /proc/self/maps",
            expected.display()
        ))
    })
}

fn original_symbol_addresses(
    object: &object::File<'_>,
) -> HashMap<String, u64> {
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

fn parse_merged_table(elf: &[u8]) -> Result<Option<Table>, InitError> {
    match Table::parse(elf) {
        Ok(table) => Ok(table),
        Err(err) => {
            let message = err.to_string();
            if message.contains("no `.defmt` section")
                || message.contains("version found, but no `.defmt` section")
            {
                Ok(None)
            } else {
                Err(InitError::ParseTable(message))
            }
        }
    }
}

fn is_defmt_section(name: &str) -> bool {
    name == ".defmt"
        || name.starts_with(".defmt.")
        || name.starts_with(".defmt,")
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

fn host_timestamp_entry() -> TableEntry {
    TableEntry::new(
        StringEntry::new(Tag::Timestamp, "{=u64:us}".to_owned()),
        "defmt2log::timestamp".to_owned(),
    )
}

fn symbol_tag(raw: &str) -> Result<Option<Tag>, InitError> {
    let Some(tag) = raw
        .split("\"tag\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
    else {
        return Err(InitError::ParseTable(format!(
            "failed to parse defmt symbol tag from `{raw}`"
        )));
    };

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
