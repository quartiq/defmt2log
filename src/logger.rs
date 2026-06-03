use defmt_parser::Level as DefmtLevel;
use log::Level;
use std::{cell::RefCell, io::Write};

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

defmt::timestamp!("");

pub(crate) fn emit(raw: &[u8]) {
    let info = crate::info();

    match info.table.decode_with_bias(raw, info.frame_index_bias) {
        Ok((frame, consumed)) if consumed == raw.len() => {
            let location = info.locations.get(&frame.index());
            match frame.level() {
                Some(level) => emit_log(&frame, level, location),
                None => print_frame(&frame, location),
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

fn emit_log(
    frame: &defmt_decoder::Frame<'_>,
    level: DefmtLevel,
    location: Option<&defmt_decoder::Location>,
) {
    let level = match level {
        DefmtLevel::Trace => Level::Trace,
        DefmtLevel::Debug => Level::Debug,
        DefmtLevel::Info => Level::Info,
        DefmtLevel::Warn => Level::Warn,
        DefmtLevel::Error => Level::Error,
    };
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
                .line(location.and_then(|l| l.line.try_into().ok()))
                .build(),
        );
    }
}

fn print_frame(frame: &defmt_decoder::Frame<'_>, location: Option<&defmt_decoder::Location>) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{}", frame.display_message()).ok();
    if let Some(location) = location {
        write!(
            stdout,
            "  at {} @ {}",
            location.module,
            location.file.display()
        )
        .ok();
        if let Ok(line) = u32::try_from(location.line) {
            write!(stdout, ":{line}").ok();
        }
        writeln!(stdout).ok();
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
            assert!(thread.raw.is_empty());
            thread.acquired = true;
        });
    }

    unsafe fn flush() {}

    unsafe fn release() {
        THREAD.with(|thread| {
            let mut thread = thread.borrow_mut();
            assert!(thread.acquired, "defmt logger released without acquire");
            emit(&thread.raw);
            thread.raw.clear();
            thread.acquired = false;
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
