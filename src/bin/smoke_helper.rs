use defmt::info;
use log::{LevelFilter, Log, Metadata, Record};

struct Logger;

static LOGGER: Logger = Logger;

impl Log for Logger {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        eprintln!(
            "{}|{}|{}|{}|{}|{}",
            record.level(),
            record.target(),
            record.module_path().unwrap_or("<unknown>"),
            record.file().unwrap_or("<unknown>"),
            record.line().unwrap_or(0),
            record.args()
        );
    }

    fn flush(&self) {}
}

fn main() {
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(LevelFilter::Trace);

    match std::env::var_os("DEFMT2LOG_SELF_CHECK") {
        Some(path) => defmt2log::init_from_elf_path(path).unwrap(),
        None => defmt2log::init_from_current_exe().unwrap(),
    }

    info!("word {=u32:#010x}", 0x1234u32);
}
