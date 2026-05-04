use defmt_parser::Level as DefmtLevel;
use log::Level;
use std::cell::RefCell;

#[derive(Default)]
struct ThreadState {
    acquired: bool,
    raw: Vec<u8>,
}

thread_local! {
    static THREAD: RefCell<ThreadState> = RefCell::new(ThreadState::default());
}

#[defmt::global_logger]
struct HostLogger;

defmt::timestamp!("{=u64:us}", 0u64);

fn emit(raw: &[u8]) {
    let state = crate::state();
    match state.table.decode(raw) {
        Ok((frame, consumed)) if consumed == raw.len() => {
            let level = match frame.level() {
                Some(DefmtLevel::Trace) => Level::Trace,
                Some(DefmtLevel::Debug) => Level::Debug,
                Some(DefmtLevel::Info) => Level::Info,
                Some(DefmtLevel::Warn) => Level::Warn,
                Some(DefmtLevel::Error) => Level::Error,
                None => Level::Info,
            };
            let location = state.locations.as_ref().and_then(|l| l.get(&frame.index()));
            let module = location.map(|l| l.module.as_str());
            let metadata = log::MetadataBuilder::new()
                .level(level)
                .target(module.unwrap_or("defmt"))
                .build();
            let logger = log::logger();
            if logger.enabled(&metadata) {
                logger.log(
                    &log::Record::builder()
                        .args(format_args!("{}", frame.display_message()))
                        .metadata(metadata)
                        .module_path(module)
                        .file(location.and_then(|l| l.file.to_str()))
                        .line(location.and_then(|l| u32::try_from(l.line).ok()))
                        .build(),
                );
            }
        }
        Ok((_frame, consumed)) => {
            log::warn!(
                "defmt2log decoded partial frame: consumed {consumed} of {} bytes",
                raw.len()
            );
        }
        Err(defmt_decoder::DecodeError::UnexpectedEof) => {
            log::warn!("defmt2log saw incomplete raw frame of {} bytes", raw.len());
        }
        Err(defmt_decoder::DecodeError::Malformed) => {
            log::warn!(
                "defmt2log saw malformed raw frame of {} bytes: {raw:02x?}",
                raw.len(),
            );
        }
    }
}

// Safety: `defmt` serializes access through `acquire`/`write`/`release`. We
// keep one raw-frame buffer per thread and only touch it while the thread is in
// the acquired state.
unsafe impl defmt::Logger for HostLogger {
    fn acquire() {
        THREAD.with(|thread| {
            let mut thread = thread.borrow_mut();
            assert!(
                !thread.acquired,
                "defmt logger acquired twice in one thread"
            );
            thread.acquired = true;
            assert!(thread.raw.is_empty());
        });
    }

    unsafe fn flush() {}

    unsafe fn release() {
        THREAD.with(|thread| {
            let mut thread = thread.borrow_mut();
            assert!(thread.acquired, "defmt logger released without acquire");
            thread.acquired = false;
            emit(&thread.raw);
            thread.raw.clear();
        });
    }

    unsafe fn write(bytes: &[u8]) {
        THREAD.with(|thread| {
            let mut thread = thread.borrow_mut();
            assert!(thread.acquired, "defmt logger write without acquire");
            thread.raw.extend_from_slice(bytes);
        });
    }
}
