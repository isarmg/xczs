#[cfg(test)]
use anyhow::bail;
use anyhow::{Context, Result};
use log::{Level, LevelFilter, Metadata, Record};
#[cfg(test)]
use rustix::{
    fs::{FileType, Mode, OFlags, fchmod, fstat, open},
    io::Errno,
    process::geteuid,
};
#[cfg(test)]
use std::fs::File;
#[cfg(test)]
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
    mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
};
use std::thread;
use std::time::{Duration, Instant};

// Request threads must never wait for a slow terminal, pipe or filesystem.
// When this bounded queue is full, the newest entry is dropped and the writer
// emits one aggregate warning as soon as it can make progress again.
const LOG_QUEUE_CAPACITY: usize = 4096;
const LOG_FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const LOG_FLUSH_DEADLINE: Duration = Duration::from_secs(5);
const LOG_DROP_REPORT_INTERVAL: Duration = Duration::from_secs(1);
pub(crate) const MAX_LOG_ENTRY_BYTES: usize = 16 * 1024;
pub(crate) const LOG_TRUNCATION_SUFFIX: &str = "...[truncated]";

/// Incrementally build one bounded log entry without first allocating the
/// potentially much larger untruncated value.
pub(crate) struct BoundedLogLine {
    value: String,
    truncated: bool,
}

impl BoundedLogLine {
    pub(crate) fn new() -> Self {
        Self {
            value: String::new(),
            truncated: false,
        }
    }

    pub(crate) fn push_str(&mut self, value: &str) {
        if self.truncated {
            return;
        }
        if value.len() <= MAX_LOG_ENTRY_BYTES.saturating_sub(self.value.len()) {
            self.value.push_str(value);
            return;
        }

        let content_limit = MAX_LOG_ENTRY_BYTES - LOG_TRUNCATION_SUFFIX.len();
        if self.value.len() > content_limit {
            let mut end = content_limit;
            while !self.value.is_char_boundary(end) {
                end -= 1;
            }
            self.value.truncate(end);
        }

        let mut end = value.len().min(content_limit - self.value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.value.push_str(&value[..end]);
        self.value.push_str(LOG_TRUNCATION_SUFFIX);
        self.truncated = true;
    }

    pub(crate) fn finish(self) -> String {
        self.value
    }

    pub(crate) fn is_truncated(&self) -> bool {
        self.truncated
    }
}

static HTTP_LOG_QUEUE: OnceLock<(SyncSender<WriterCommand>, Arc<AtomicU64>)> = OnceLock::new();

/// HTTP request context is attached to the typed record before queueing; it
/// never has to be recovered from a formatted access-log message.
pub(crate) fn emit_http_record(access: &str, request_id: Option<&str>, is_error: bool) -> bool {
    if !log::log_enabled!(if is_error { Level::Error } else { Level::Info }) {
        return true;
    }
    let Some((sender, dropped)) = HTTP_LOG_QUEUE.get() else {
        return false;
    };
    let level = if is_error {
        xcss::log::Level::Error
    } else {
        xcss::log::Level::Info
    };
    let record = xcss::log::LogRecord::server(
        "xczs",
        "http",
        "xczs.http.completed",
        "HTTP request completed.",
        level,
    )
    .and_then(|record| {
        record.with_attribute("access", truncate_log_entry(sanitize_log_line(access)))
    })
    .and_then(|record| match request_id {
        Some(id) => record.with_request_id(id),
        None => Ok(record),
    });
    match record {
        Ok(record) => {
            if sender
                .try_send(WriterCommand::Entry(Box::new(LogEntry { record })))
                .is_err()
            {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        Err(_) => {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    true
}

struct AsyncLogger {
    level: LevelFilter,
    sender: SyncSender<WriterCommand>,
    dropped: Arc<AtomicU64>,
}

enum WriterCommand {
    Entry(Box<LogEntry>),
    Flush(std::sync::mpsc::Sender<()>),
}

struct LogEntry {
    record: xcss::log::LogRecord,
}

enum LogOutput {
    #[cfg(test)]
    File(BufWriter<File>),
    Process,
}

impl LogOutput {
    fn write_line(&mut self, entry: &LogEntry) -> std::io::Result<()> {
        match self {
            #[cfg(test)]
            Self::File(file) => entry.record.write_to(file).map_err(std::io::Error::other),
            Self::Process => entry.record.emit().map_err(std::io::Error::other),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(test)]
            Self::File(file) => file.flush(),
            Self::Process => Ok(()),
        }
    }
}

impl log::Log for AsyncLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let level = match record.level() {
            Level::Error => xcss::log::Level::Error,
            Level::Warn => xcss::log::Level::Warn,
            Level::Info => xcss::log::Level::Info,
            Level::Debug => xcss::log::Level::Debug,
            Level::Trace => xcss::log::Level::Trace,
        };
        let result = match record.target() {
            "common.config.loaded" => {
                xcss::log::LogRecord::common("xczs", xcss::log::CommonEvent::ConfigLoaded)
            }
            "common.runtime.started" => {
                xcss::log::LogRecord::common("xczs", xcss::log::CommonEvent::RuntimeStarted)
            }
            "common.runtime.stopped" => {
                xcss::log::LogRecord::common("xczs", xcss::log::CommonEvent::RuntimeStopped)
            }
            "common.runtime.shutdown_started" => {
                xcss::log::LogRecord::common("xczs", xcss::log::CommonEvent::ShutdownStarted)
            }
            "http_access" => xcss::log::LogRecord::server(
                "xczs",
                "http",
                "xczs.http.completed",
                "HTTP request completed.",
                level,
            )
            .and_then(|value| {
                value.with_attribute(
                    "access",
                    truncate_log_entry(sanitize_log_line(&record.args().to_string())),
                )
            }),
            _ => xcss::log::LogRecord::server(
                "xczs",
                if record.target().is_empty() {
                    "runtime"
                } else {
                    record.target()
                },
                "xczs.diagnostic",
                "Runtime diagnostic event.",
                level,
            ),
        };
        let entry = match result {
            Ok(record) => LogEntry { record },
            Err(_) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        match self.sender.try_send(WriterCommand::Entry(Box::new(entry))) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn flush(&self) {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let deadline = Instant::now() + LOG_FLUSH_DEADLINE;
        let mut command = WriterCommand::Flush(done_tx);
        loop {
            match self.sender.try_send(command) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) if Instant::now() < deadline => {
                    command = returned;
                    thread::sleep(Duration::from_millis(1));
                }
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => return,
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            let _ = done_rx.recv_timeout(remaining);
        }
    }
}

pub fn init(root: &Path, level: LevelFilter) -> Result<()> {
    xcss::fs_safety::PrivateDirectory::open_existing(root.join("logs"))?;
    let path = root.join("logs/xczs.jsonl");
    xcss::log::install_rotating_file(xcss::log::RotatingLogFile::open_file(
        path,
        xcss::log::LogRetention::default(),
    )?)?;
    let output = LogOutput::Process;

    let (sender, receiver) = sync_channel(LOG_QUEUE_CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));
    let writer_dropped = dropped.clone();
    thread::Builder::new()
        .name("xczs-log-writer".to_string())
        .spawn(move || writer_loop(receiver, output, &writer_dropped))
        .context("Failed to start the log writer")?;

    HTTP_LOG_QUEUE
        .set((sender.clone(), dropped.clone()))
        .map_err(|_| anyhow::anyhow!("HTTP log queue is already initialized"))?;
    let logger = AsyncLogger {
        sender,
        dropped,
        level,
    };
    log::set_boxed_logger(Box::new(logger))
        .map(|_| log::set_max_level(level))
        .with_context(|| "Failed to init logger")?;
    Ok(())
}

#[cfg(test)]
fn open_log_file(path: &Path) -> Result<File> {
    open_log_file_for_owner(path, geteuid().as_raw())
}

#[cfg(test)]
fn open_log_file_for_owner(path: &Path, expected_owner: u32) -> Result<File> {
    let private_mode = Mode::RUSR | Mode::WUSR;
    let common_flags =
        OFlags::WRONLY | OFlags::APPEND | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let (fd, created) = match open(
        path,
        common_flags | OFlags::CREATE | OFlags::EXCL,
        private_mode,
    ) {
        Ok(fd) => (fd, true),
        Err(Errno::EXIST) => (
            open(path, common_flags, Mode::empty())
                .map_err(std::io::Error::from)
                .with_context(|| {
                    format!("Failed to securely open log file '{}'", path.display())
                })?,
            false,
        ),
        Err(error) => {
            return Err(std::io::Error::from(error)).with_context(|| {
                format!("Failed to securely create log file '{}'", path.display())
            });
        }
    };

    let metadata = fstat(&fd)
        .map_err(std::io::Error::from)
        .with_context(|| format!("Failed to inspect log file '{}'", path.display()))?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile {
        bail!("Log file '{}' must be a regular file", path.display());
    }
    if metadata.st_nlink != 1 {
        bail!(
            "Log file '{}' must have exactly one hard link",
            path.display()
        );
    }
    if metadata.st_uid != expected_owner {
        bail!(
            "Log file '{}' must be owned by the effective service user",
            path.display()
        );
    }

    if created {
        fchmod(&fd, private_mode)
            .map_err(std::io::Error::from)
            .with_context(|| {
                format!(
                    "Failed to set private log permissions on '{}'",
                    path.display()
                )
            })?;
    } else if Mode::from_raw_mode(metadata.st_mode) != private_mode {
        bail!(
            "Existing log file '{}' must already have permissions 0600; refusing to change insecure permissions after opening it",
            path.display()
        );
    }
    let verified = fstat(&fd)
        .map_err(std::io::Error::from)
        .with_context(|| format!("Failed to verify log file '{}'", path.display()))?;
    if FileType::from_raw_mode(verified.st_mode) != FileType::RegularFile
        || verified.st_nlink != 1
        || verified.st_uid != expected_owner
        || Mode::from_raw_mode(verified.st_mode) != private_mode
    {
        bail!(
            "Log file '{}' changed while its security properties were being verified",
            path.display()
        );
    }

    Ok(fd.into())
}

fn writer_loop(receiver: Receiver<WriterCommand>, mut output: LogOutput, dropped: &AtomicU64) {
    let mut dirty = false;
    let mut next_flush = Instant::now() + LOG_FLUSH_INTERVAL;
    let mut last_drop_report = Instant::now()
        .checked_sub(LOG_DROP_REPORT_INTERVAL)
        .unwrap_or_else(Instant::now);
    loop {
        let wait = next_flush.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(wait) {
            Ok(WriterCommand::Entry(entry)) => {
                dirty |= report_dropped_if_due(
                    &mut output,
                    dropped,
                    &mut last_drop_report,
                    Instant::now(),
                    false,
                );
                if let Err(error) = output.write_line(&entry) {
                    report_internal_log_error("log_writer_error", &error);
                } else {
                    dirty = true;
                }
            }
            Ok(WriterCommand::Flush(done)) => {
                dirty |= report_dropped_if_due(
                    &mut output,
                    dropped,
                    &mut last_drop_report,
                    Instant::now(),
                    true,
                );
                if let Err(error) = flush_if_dirty(&mut dirty, || output.flush()) {
                    report_internal_log_error("log_flush_error", &error);
                }
                next_flush = Instant::now() + LOG_FLUSH_INTERVAL;
                let _ = done.send(());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if Instant::now() >= next_flush {
            dirty |= report_dropped_if_due(
                &mut output,
                dropped,
                &mut last_drop_report,
                Instant::now(),
                false,
            );
            if let Err(error) = flush_if_dirty(&mut dirty, || output.flush()) {
                report_internal_log_error("log_flush_error", &error);
            }
            next_flush = Instant::now() + LOG_FLUSH_INTERVAL;
        }
    }

    report_dropped(&mut output, dropped);
    let _ = output.flush();
}

fn flush_if_dirty(
    dirty: &mut bool,
    flush: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    if !*dirty {
        return Ok(());
    }
    flush()?;
    *dirty = false;
    Ok(())
}

fn report_internal_log_error(event: &str, error: &std::io::Error) {
    let mut stderr = std::io::stderr().lock();
    let _ = write_internal_log_error(&mut stderr, event, error);
}

fn write_internal_log_error(
    output: &mut impl Write,
    event: &str,
    error: &std::io::Error,
) -> std::io::Result<()> {
    let _ = error;
    let record = xcss::log::LogRecord::server(
        "xczs",
        "logging",
        &format!("xczs.{event}"),
        "The runtime log sink is unavailable.",
        xcss::log::Level::Error,
    )
    .map_err(std::io::Error::other)?;
    record.write_to(output).map_err(|error| match error {
        xcss::log::LogError::Io(error) => error,
        other => std::io::Error::other(other),
    })
}

fn report_dropped_if_due(
    output: &mut LogOutput,
    dropped: &AtomicU64,
    last_report: &mut Instant,
    now: Instant,
    force: bool,
) -> bool {
    if !force && now.saturating_duration_since(*last_report) < LOG_DROP_REPORT_INTERVAL {
        return false;
    }
    let reported = report_dropped(output, dropped);
    if reported {
        *last_report = now;
    }
    reported
}

fn report_dropped(output: &mut LogOutput, dropped: &AtomicU64) -> bool {
    let count = dropped.swap(0, Ordering::Relaxed);
    if count == 0 {
        return false;
    }
    let warning = LogEntry {
        record: xcss::log::LogRecord::server(
            "xczs",
            "logging",
            "xczs.log_queue_overloaded",
            "The bounded log queue dropped new records.",
            xcss::log::Level::Warn,
        )
        .and_then(|value| value.with_attribute("dropped_newest", count))
        .and_then(|value| value.with_attribute("capacity", LOG_QUEUE_CAPACITY))
        .expect("static bounded log record"),
    };
    match output.write_line(&warning) {
        Ok(()) => true,
        Err(_) => {
            dropped.fetch_add(count, Ordering::Relaxed);
            false
        }
    }
}

fn truncate_log_entry(mut value: String) -> String {
    if value.len() <= MAX_LOG_ENTRY_BYTES {
        return value;
    }

    let mut end = MAX_LOG_ENTRY_BYTES.saturating_sub(LOG_TRUNCATION_SUFFIX.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str(LOG_TRUNCATION_SUFFIX);
    value
}

/// Convert arbitrary text to one physical log line.
///
/// Structured access-log fields may add their own quoting, but every log
/// record passes through this final boundary so errors from external crates
/// and operating-system strings cannot inject extra records.
pub fn sanitize_log_line(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{{{:x}}}", character as u32);
            }
            character => output.push(character),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log as _;
    use std::io;
    use std::os::unix::fs::{PermissionsExt, symlink};

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "diagnostic sink is closed",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn internal_log_error_write_failure_is_returned_without_panicking() {
        let source = io::Error::other("original write failure");
        let mut output = FailingWriter;

        let error = write_internal_log_error(&mut output, "log_writer_error", &source)
            .expect_err("closed diagnostic sink unexpectedly accepted the fallback log");

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn failed_flush_stays_dirty_until_a_retry_succeeds() {
        let mut dirty = true;
        let mut attempts = 0;

        let error = flush_if_dirty(&mut dirty, || {
            attempts += 1;
            Err(io::Error::other("first flush failed"))
        })
        .expect_err("failed flush unexpectedly succeeded");
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(dirty, "a failed flush discarded the pending state");

        flush_if_dirty(&mut dirty, || {
            attempts += 1;
            Ok(())
        })
        .expect("flush retry failed");
        assert!(!dirty, "a successful flush left stale pending state");
        assert_eq!(attempts, 2);
    }

    #[test]
    fn arbitrary_values_are_reduced_to_one_physical_line() {
        assert_eq!(
            sanitize_log_line("first\r\nsecond\t\u{7f}"),
            "first\\r\\nsecond\\t\\u{7f}"
        );
    }

    #[test]
    fn queued_log_entries_have_a_utf8_safe_byte_limit() {
        let exact = "x".repeat(MAX_LOG_ENTRY_BYTES);
        assert_eq!(truncate_log_entry(exact.clone()), exact);

        let oversized = format!("prefix-{}", "中文".repeat(MAX_LOG_ENTRY_BYTES));
        let truncated = truncate_log_entry(oversized);
        assert!(truncated.len() <= MAX_LOG_ENTRY_BYTES);
        assert!(truncated.ends_with(LOG_TRUNCATION_SUFFIX));
        assert!(std::str::from_utf8(truncated.as_bytes()).is_ok());
    }

    #[test]
    fn log_files_are_created_private_and_existing_private_files_are_appended() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let path = temporary.path().join("xczs.log");
        let mut file = open_log_file(&path).expect("securely create log");
        file.write_all(b"created\n").expect("write first log entry");
        file.flush().expect("flush first log entry");
        drop(file);
        assert_eq!(
            std::fs::metadata(&path)
                .expect("inspect created log")
                .permissions()
                .mode()
                & 0o7777,
            0o600
        );

        let mut file = open_log_file(&path).expect("securely open existing log");
        file.write_all(b"appended\n").expect("append log entry");
        file.flush().expect("flush log entry");
        drop(file);

        let metadata = std::fs::metadata(&path).expect("inspect log");
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
        assert_eq!(
            std::fs::read_to_string(&path).expect("read log"),
            "created\nappended\n"
        );
    }

    #[test]
    fn insecure_existing_log_permissions_are_rejected_without_modification() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        for mode in [0o644, 0o660, 0o604] {
            let path = temporary.path().join(format!("xczs-{mode:o}.log"));
            std::fs::write(&path, b"existing\n").expect("create existing log");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .expect("set insecure initial permissions");

            let error = open_log_file(&path).expect_err("insecure log mode was accepted");
            assert!(
                error
                    .to_string()
                    .contains("must already have permissions 0600"),
                "unexpected error for mode {mode:o}: {error:#}"
            );
            assert_eq!(
                std::fs::metadata(&path)
                    .expect("inspect rejected log")
                    .permissions()
                    .mode()
                    & 0o7777,
                mode
            );
            assert_eq!(
                std::fs::read(&path).expect("read rejected log"),
                b"existing\n"
            );
        }
    }

    #[test]
    fn log_file_symlinks_are_rejected_without_touching_the_target() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let target = temporary.path().join("target");
        let link = temporary.path().join("xczs.log");
        std::fs::write(&target, b"must remain unchanged").expect("create target");
        symlink(&target, &link).expect("create log symlink");

        let error = open_log_file(&link).expect_err("log symlink was accepted");
        assert!(
            format!("{error:#}").contains("securely open"),
            "unexpected error: {error:#}"
        );
        assert_eq!(
            std::fs::read(&target).expect("read target"),
            b"must remain unchanged"
        );
    }

    #[test]
    fn multiply_linked_and_unexpected_owner_log_files_are_rejected() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let original = temporary.path().join("original");
        let linked = temporary.path().join("xczs.log");
        std::fs::write(&original, b"existing").expect("create original");
        std::fs::hard_link(&original, &linked).expect("create hard link");
        let error = open_log_file(&linked).expect_err("multiply linked log was accepted");
        assert!(
            error.to_string().contains("exactly one hard link"),
            "unexpected error: {error:#}"
        );

        let owner_path = temporary.path().join("owner.log");
        std::fs::write(&owner_path, b"existing").expect("create owner test log");
        let current_owner = geteuid().as_raw();
        let unexpected_owner = if current_owner == 0 { 1 } else { 0 };
        let error = open_log_file_for_owner(&owner_path, unexpected_owner)
            .expect_err("unexpected log owner was accepted");
        assert!(
            error.to_string().contains("effective service user"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn dropped_warning_is_rate_limited_but_force_flush_reports_pending_count() {
        let temporary = tempfile::NamedTempFile::new().expect("create temporary log");
        let mut output = LogOutput::File(BufWriter::new(
            temporary.reopen().expect("reopen temporary log"),
        ));
        let dropped = AtomicU64::new(3);
        let now = Instant::now();
        let mut last_report = now;

        assert!(!report_dropped_if_due(
            &mut output,
            &dropped,
            &mut last_report,
            now,
            false,
        ));
        assert_eq!(dropped.load(Ordering::Relaxed), 3);

        assert!(report_dropped_if_due(
            &mut output,
            &dropped,
            &mut last_report,
            now + LOG_DROP_REPORT_INTERVAL,
            false,
        ));
        dropped.store(2, Ordering::Relaxed);
        assert!(report_dropped_if_due(
            &mut output,
            &dropped,
            &mut last_report,
            now + LOG_DROP_REPORT_INTERVAL,
            true,
        ));
        output.flush().expect("flush temporary log");

        let contents = std::fs::read_to_string(temporary.path()).expect("read temporary log");
        assert_eq!(contents.matches("\"dropped_newest\":").count(), 2);
        assert!(contents.contains("\"dropped_newest\":3"));
        assert!(contents.contains("\"dropped_newest\":2"));
    }

    #[test]
    fn full_queue_drops_the_newest_entry_without_blocking() {
        let (sender, receiver) = sync_channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let logger = AsyncLogger {
            level: LevelFilter::Trace,
            sender,
            dropped: dropped.clone(),
        };

        logger.log(
            &Record::builder()
                .args(format_args!("first"))
                .level(Level::Info)
                .build(),
        );
        logger.log(
            &Record::builder()
                .args(format_args!("second"))
                .level(Level::Info)
                .build(),
        );

        assert!(matches!(
            receiver.try_recv(),
            Ok(WriterCommand::Entry(entry)) if serde_json::to_string(&entry.record).unwrap().contains("Runtime diagnostic event.")
        ));
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn flush_waits_for_queued_entries_to_be_written() {
        let temporary = tempfile::NamedTempFile::new().expect("create temporary log");
        let output = LogOutput::File(BufWriter::new(
            temporary.reopen().expect("reopen temporary log"),
        ));
        let (sender, receiver) = sync_channel(2);
        let dropped = Arc::new(AtomicU64::new(0));
        let writer_dropped = dropped.clone();
        let writer = thread::spawn(move || writer_loop(receiver, output, &writer_dropped));
        let logger = AsyncLogger {
            sender,
            dropped,
            level: LevelFilter::Trace,
        };

        logger.log(
            &Record::builder()
                .args(format_args!("durable line"))
                .level(Level::Info)
                .build(),
        );
        logger.flush();
        assert!(
            std::fs::read_to_string(temporary.path())
                .expect("read temporary log")
                .contains("Runtime diagnostic event.")
        );

        drop(logger);
        writer.join().expect("join log writer");
    }
}
