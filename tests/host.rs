use std::sync::Mutex;

use log::{Level, LevelFilter, Metadata, Record};

defmt::timestamp!("{=u32}", 0x1234_5678u32);

defmt::bitflags! {
    struct Flags: u8 {
        const A = 1;
        const B = 2;
    }
}

static LOGGER: TestLogger = TestLogger {
    records: Mutex::new(Vec::new()),
};

struct TestLogger {
    records: Mutex<Vec<(Level, String)>>,
}

impl TestLogger {
    fn install(&'static self) {
        if log::set_logger(self).is_ok() {
            log::set_max_level(LevelFilter::Trace);
        }
        self.records.lock().unwrap().clear();
    }

    fn take(&self) -> Vec<(Level, String)> {
        self.records.lock().unwrap().drain(..).collect()
    }
}

impl log::Log for TestLogger {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        self.records
            .lock()
            .unwrap()
            .push((record.level(), record.args().to_string()));
    }

    fn flush(&self) {}
}

#[test]
fn defmt_records_are_forwarded_to_log() {
    defmt2log::init();
    LOGGER.install();

    defmt::error!("word {=u32:#010x}", 0x1234u32);
    defmt::error!("flags {}", Flags::A | Flags::B);

    let records = LOGGER.take();
    let [(word_level, word_message), (flags_level, flags_message)] = records.as_slice() else {
        panic!("unexpected log records: {records:?}");
    };
    assert_eq!(*word_level, Level::Error);
    assert_eq!(word_message, "word 0x00001234");
    assert_eq!(*flags_level, Level::Error);
    assert_eq!(flags_message, "flags A | B");
}
