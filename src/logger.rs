use std::cell::RefCell;

use defmt::Logger;
use defmt_decoder::{DecodeError, Frame, Location};
use log::Level;

use crate::state;

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

fn level(frame: &Frame<'_>) -> Level {
    match frame.level() {
        Some(defmt_parser::Level::Trace) => Level::Trace,
        Some(defmt_parser::Level::Debug) => Level::Debug,
        Some(defmt_parser::Level::Info) => Level::Info,
        Some(defmt_parser::Level::Warn) => Level::Warn,
        Some(defmt_parser::Level::Error) => Level::Error,
        None => Level::Info,
    }
}

fn location<'a>(frame: &Frame<'_>) -> Option<&'a Location> {
    state()
        .locations
        .as_ref()
        .and_then(|locations| locations.get(&frame.index()))
}

fn emit(raw: &[u8]) {
    let state = state();
    match state.table.decode(raw) {
        Ok((frame, consumed)) if consumed == raw.len() => {
            let location = location(&frame);
            let module = location.map(|location| location.module.as_str());
            let file = location.and_then(|location| location.file.to_str());
            let line = location.and_then(|location| u32::try_from(location.line).ok());
            let level = level(&frame);
            let message = frame.display_message().to_string();
            let args = format_args!("{message}");
            log::logger().log(
                &log::Record::builder()
                    .args(args)
                    .level(level)
                    .target(module.unwrap_or("defmt"))
                    .module_path(module)
                    .file(file)
                    .line(line)
                    .build(),
            );
        }
        Ok((_frame, consumed)) => {
            log::warn!(
                "defmt2log decoded partial frame: consumed {consumed} of {} bytes",
                raw.len()
            );
        }
        Err(DecodeError::UnexpectedEof) => {
            log::warn!("defmt2log saw incomplete raw frame of {} bytes", raw.len());
        }
        Err(DecodeError::Malformed) => {
            log::warn!(
                "defmt2log saw malformed raw frame of {} bytes: {:02x?}; indices={:?}; has_timestamp={}",
                raw.len(),
                raw,
                state.table.indices().collect::<Vec<_>>(),
                state.table.has_timestamp()
            );
        }
    }
}

// Safety: `defmt` serializes access through `acquire`/`write`/`release`. We
// keep one raw-frame buffer per thread and only touch it while the thread is in
// the acquired state.
unsafe impl Logger for HostLogger {
    fn acquire() {
        THREAD.with(|thread| {
            let mut thread = thread.borrow_mut();
            assert!(
                !thread.acquired,
                "defmt logger acquired twice in one thread"
            );
            thread.acquired = true;
            thread.raw.clear();
        });
    }

    unsafe fn flush() {}

    unsafe fn release() {
        THREAD.with(|thread| {
            let raw = {
                let mut thread = thread.borrow_mut();
                assert!(thread.acquired, "defmt logger released without acquire");
                thread.acquired = false;
                std::mem::take(&mut thread.raw)
            };
            emit(&raw);
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
