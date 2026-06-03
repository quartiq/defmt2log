#![doc = include_str!("../README.md")]

use std::{env, error::Error, fs, path::Path, sync::OnceLock};

use defmt_decoder::{Locations, Table};

mod logger;
mod table;

pub(crate) struct Info {
    pub(crate) table: Table,
    pub(crate) locations: Locations,
    pub(crate) frame_index_bias: u16,
}

static INFO: OnceLock<Info> = OnceLock::new();

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

fn init_state(state: Result<Info, Box<dyn Error>>) {
    let state = state.unwrap_or_else(|err| panic!("defmt2log initialization failed: {err}"));
    INFO.set(state)
        .unwrap_or_else(|_| panic!("defmt2log is already initialized"));
}

pub(crate) fn info() -> &'static Info {
    INFO.get()
        .expect("defmt2log must be initialized before emitting defmt logs")
}

#[cfg(test)]
mod test {
    use std::{
        env,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    use defmt_decoder::{Location, Locations, Table};
    use log::{Level, LevelFilter, Metadata, Record};

    static LOGGER: TestLogger = TestLogger {
        enabled: AtomicBool::new(true),
        records: Mutex::new(Vec::new()),
    };

    #[derive(Debug)]
    struct Snapshot {
        level: Level,
        target: String,
        module_path: Option<String>,
        file: Option<String>,
        line: Option<u32>,
        message: String,
    }

    struct TestLogger {
        enabled: AtomicBool,
        records: Mutex<Vec<Snapshot>>,
    }

    impl TestLogger {
        fn install(&'static self) {
            if log::set_logger(self).is_ok() {
                log::set_max_level(LevelFilter::Trace);
            }
            self.records.lock().unwrap().clear();
            self.enabled.store(true, Ordering::Relaxed);
        }

        fn set_enabled(&self, enabled: bool) {
            self.enabled.store(enabled, Ordering::Relaxed);
        }

        fn take(&self) -> Vec<Snapshot> {
            self.records.lock().unwrap().drain(..).collect()
        }
    }

    impl log::Log for TestLogger {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            self.enabled.load(Ordering::Relaxed)
        }

        fn log(&self, record: &Record<'_>) {
            self.records.lock().unwrap().push(Snapshot {
                level: record.level(),
                target: record.target().to_string(),
                module_path: record.module_path().map(ToString::to_string),
                file: record.file().map(ToString::to_string),
                line: record.line(),
                message: record.args().to_string(),
            });
        }

        fn flush(&self) {}
    }

    #[test]
    fn host_logging_decodes_records_and_prints() {
        let exe = env::current_exe().unwrap();
        let elf = super::read_elf(&exe);
        super::table::load_host_state(&elf, &exe).unwrap();

        LOGGER.install();
        super::INFO.set(test_info()).ok().unwrap();

        super::logger::emit(&[4, 0, 0x34, 0x12, 0, 0]);
        let records = LOGGER.take();
        let [record] = records.as_slice() else {
            panic!("unexpected defmt records: {records:?}");
        };
        assert_eq!(record.level, Level::Info);
        assert_eq!(record.message, "word 0x00001234");
        assert_eq!(record.target, "defmt2log::test::callsite");
        assert_eq!(record.target, record.module_path.as_deref().unwrap());
        assert!(record.file.as_deref().unwrap().ends_with("src/lib.rs"));
        assert_eq!(record.line, Some(123));

        LOGGER.set_enabled(false);
        super::logger::emit(&[4, 0, 0x34, 0x12, 0, 0]);
        super::logger::emit(&[5, 0, 7]);
        assert!(LOGGER.take().is_empty());
    }

    fn test_info() -> super::Info {
        let table = serde_json::from_str::<Table>(
            r#"{
                "timestamp": null,
                "entries": {
                    "1": {
                        "string": { "tag": "Info", "string": "word {=u32:#010x}" },
                        "raw_symbol": "info"
                    },
                    "2": {
                        "string": { "tag": "Println", "string": "always printed {=u8}" },
                        "raw_symbol": "println"
                    }
                },
                "bitflags": {},
                "encoding": "Raw"
            }"#,
        )
        .unwrap();
        let locations = Locations::from([(
            1,
            Location {
                file: "src/lib.rs".into(),
                line: 123,
                module: "defmt2log::test::callsite".to_string(),
            },
        )]);

        super::Info {
            table,
            locations,
            frame_index_bias: 3,
        }
    }

    #[allow(dead_code)]
    fn host_metadata_marker() {
        defmt::println!("host metadata marker");
    }
}
