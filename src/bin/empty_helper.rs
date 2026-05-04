use log::{LevelFilter, Log, Metadata, Record};

struct Logger;

static LOGGER: Logger = Logger;

impl Log for Logger {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        eprintln!("{}", record.args());
    }

    fn flush(&self) {}
}

fn main() {
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(LevelFilter::Trace);

    let _ = core::mem::size_of::<defmt::Str>();
    defmt2log::init_from_current_exe().unwrap();
}
