use std::sync::Mutex;

use log::{Level, LevelFilter, Metadata, Record};

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
    LOGGER.install();
    defmt2log::init();

    defmt::error!("word {=u32:#010x}", 0x1234u32);

    let records = LOGGER.take();
    let [(level, message)] = records.as_slice() else {
        panic!("unexpected log records: {records:?}");
    };
    assert_eq!(*level, Level::Error);
    assert_eq!(message, "word 0x00001234");
}
