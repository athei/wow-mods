//! Process-lifetime logging worker owned by wow_turbo's PE image.
//!
//! Ordinary `log` records borrow their arguments and must become owned text
//! before returning to the caller. [`defer`] instead takes an owned closure:
//! it constructs formatting arguments and emits the record on the worker.
//! No Lua state or client-memory borrow is carried across that boundary.
//!
//! Installation creates no thread. A verified runtime hook calls [`start`]
//! outside the loader lock. Until then, or if spawning fails, output stays
//! synchronous so startup logs and skipped hooks cannot leave an undrained
//! queue. The installed client hooks already require this DLL to live until
//! process exit. Crash output keeps its separate direct-write path; abrupt
//! termination can lose ordinary records still in this queue.

use std::{
    cell::Cell,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    time::{Duration, Instant},
};

use log::{Level, Log, Metadata, Record};

static BACKEND: LazyLock<Backend> = LazyLock::new(|| {
    let logger = wow_shared::logger_backend_to(Box::new(crate::log_file::FileSink));
    Backend {
        maximum: logger.filter(),
        logger: Box::new(logger),
    }
});

struct Backend {
    maximum: log::LevelFilter,
    logger: Box<dyn Log>,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static QUEUE: LazyLock<Queue> = LazyLock::new(Queue::new);

thread_local! {
    /// Reports emitted by the worker go straight to its backend, never back into its queue.
    static ON_WORKER: Cell<bool> = const { Cell::new(false) };
}

/// The diagnostic reporter's cadence, independent of rendering or queue traffic.
const REPORT_INTERVAL: Duration = Duration::from_secs(60);

struct Queue {
    sender: Sender<Message>,
    receiver: Mutex<Option<Receiver<Message>>>,
}

impl Queue {
    fn new() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    }
}

enum Message {
    Text {
        level: Level,
        target: String,
        text: String,
    },
    Deferred(Box<dyn FnOnce() + Send>),
}

impl Message {
    fn emit(self, backend: &dyn Log) {
        match self {
            Self::Text {
                level,
                target,
                text,
            } => backend.log(
                &Record::builder()
                    .level(level)
                    .target(&target)
                    .args(format_args!("{text}"))
                    .build(),
            ),
            Self::Deferred(task) => task(),
        }
    }
}

struct Logger;

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        BACKEND.logger.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        let backend = &BACKEND.logger;
        if !backend.enabled(record.metadata()) {
            return;
        }
        if !active() || ON_WORKER.try_with(Cell::get).unwrap_or(false) {
            backend.log(record);
            return;
        }
        submit(Message::Text {
            level: record.level(),
            target: record.target().to_owned(),
            text: record.args().to_string(),
        });
    }

    fn flush(&self) {
        // Never wait for the worker from a client callback or DllMain.
    }
}

/// Install this DLL's facade without starting a worker under the loader lock.
pub fn init() {
    let maximum = BACKEND.maximum;
    if log::set_logger(&Logger).is_ok() {
        log::set_max_level(maximum);
    }
}

/// Whether the process-lifetime worker is consuming the queue.
#[must_use]
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Run an owned logging closure on the worker, filtering before allocation.
///
/// `Send + 'static` prevents retaining a temporary borrow. Copy transient client
/// data before capture; static literals and moved owned values need no clone.
/// Before the worker starts, the closure runs synchronously to retain startup
/// messages. Calls already on the worker execute directly, avoiding requeueing.
#[cold]
pub fn defer(level: Level, target: &str, task: impl FnOnce() + Send + 'static) {
    if !log::log_enabled!(target: target, level) {
        return;
    }
    if active() && !ON_WORKER.try_with(Cell::get).unwrap_or(false) {
        submit(Message::Deferred(Box::new(task)));
    } else {
        task();
    }
}

/// Defer the existing logging macro without rebuilding its format arguments.
///
/// Capture expressions must own their data. Snapshot mutable state before this
/// call; expressions inside the macro run later on the worker.
#[macro_export]
macro_rules! defer_log {
    (target: $target:expr, $level:expr, $($args:tt)*) => {{
        let level = $level;
        let target = $target;
        if ::log::log_enabled!(target: target, level) {
            $crate::log_worker::defer(level, target, move || {
                ::log::log!(target: target, level, $($args)*);
            });
        }
    }};
}

/// Preserve the once-per-site latch on the caller, before queue allocation.
#[macro_export]
macro_rules! defer_once_warn {
    (target: $target:expr, $($args:tt)*) => {{
        static FIRED: ::core::sync::atomic::AtomicBool = ::core::sync::atomic::AtomicBool::new(false);
        if !FIRED.swap(true, ::core::sync::atomic::Ordering::Relaxed) {
            $crate::defer_log!(target: $target, ::log::Level::Warn, $($args)*);
        }
    }};
}

fn submit(message: Message) {
    if let Err(error) = QUEUE.sender.send(message) {
        // The consumer has stopped. Do not silently retain an unbounded backlog.
        error.0.emit(BACKEND.logger.as_ref());
    }
}

/// Start once from a verified client runtime entry, never from `DllMain`.
///
/// `report` may only read thread-safe telemetry, not game-thread client state.
pub fn start(report: fn()) {
    if STARTED.load(Ordering::Relaxed) || STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let receiver = QUEUE.receiver.lock().expect("log receiver poisoned").take();
    let Some(receiver) = receiver else {
        return;
    };
    if let Err(error) = std::thread::Builder::new()
        .name("wow-turbo-log".into())
        .spawn(move || {
            ON_WORKER.set(true);
            let _lifetime = WorkerLifetime;
            ACTIVE.store(true, Ordering::Release);
            drain(&receiver, report);
        })
    {
        log::warn!(target: "wow", "log worker: {error}; retaining synchronous output");
    }
}

struct WorkerLifetime;

impl Drop for WorkerLifetime {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

fn drain(receiver: &Receiver<Message>, report: fn()) {
    let mut next_report = Instant::now() + REPORT_INTERVAL;
    loop {
        crate::log_file::maintain();
        match receiver.recv_timeout(next_report.saturating_duration_since(Instant::now())) {
            Ok(message) => {
                message.emit(BACKEND.logger.as_ref());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        if now >= next_report {
            report();
            next_report = now + REPORT_INTERVAL;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{process::Command, sync::mpsc, thread};

    use super::*;

    /// Isolate the global logging facade from the other parallel unit tests.
    #[test]
    fn worker_api_owns_arguments_and_preserves_filters() {
        fn disabled_argument() -> u32 {
            panic!("disabled argument evaluated")
        }
        const CHILD: &str = "WOW_LOG_WORKER_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let root = crate::log_file::tests::TestDir::new();
            for mode in ["file", "off", "blocked"] {
                let dir = root.0.join(mode);
                if mode == "blocked" {
                    std::fs::write(&dir, b"keep").unwrap();
                }
                let output = Command::new(std::env::current_exe().expect("test executable"))
                    .args([
                        "--exact",
                        "log_worker::tests::worker_api_owns_arguments_and_preserves_filters",
                        "--nocapture",
                    ])
                    .env(CHILD, mode)
                    .env("WOW_LOG_WORKER_TEST_DIR", &dir)
                    .env(
                        "RUST_LOG",
                        if mode == "off" {
                            "off"
                        } else {
                            "warn,wow::mpq=info"
                        },
                    )
                    .output()
                    .expect("isolated logger test");
                assert!(output.status.success(), "{output:?}");
                if mode == "off" {
                    assert!(!dir.exists());
                    assert!(output.stderr.is_empty());
                    continue;
                }
                let contents = if mode == "blocked" {
                    assert_eq!(std::fs::read(&dir).unwrap(), b"keep");
                    let text = String::from_utf8(output.stderr).unwrap();
                    assert_eq!(text.matches("using stderr").count(), 1);
                    text
                } else {
                    assert!(output.stderr.is_empty(), "{output:?}");
                    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
                    assert_eq!(files.len(), 1);
                    std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap()
                };
                assert!(contents.contains("startup record"), "{contents}");
                assert!(
                    contents.contains("owned value: decoded sector"),
                    "{contents}"
                );
                assert!(contents.contains("compatibility record"), "{contents}");
                assert!(!contents.contains("filtered record"), "{contents}");
                assert!(!contents.contains('\x1b'), "ANSI escape in plain-text log");
                assert!(contents.find("startup record") < contents.find("owned value"));
            }
            return;
        }
        init();
        if std::env::var(CHILD).unwrap() == "off" {
            crate::defer_log!(target: "wow::mpq", Level::Info, "filtered record");
            return;
        }
        assert!(!active());
        assert!(log::log_enabled!(target: "wow::mpq", Level::Info));
        assert!(!log::log_enabled!(target: "wow::other", Level::Info));
        crate::defer_log!(target: "wow::mpq", Level::Info, "startup record");
        start(|| {});
        let deadline = Instant::now() + Duration::from_secs(5);
        while !active() {
            assert!(Instant::now() < deadline, "worker failed to start");
            thread::yield_now();
        }
        start(|| panic!("second start replaced reporter"));
        let caller = thread::current().id();
        let (sender, receiver) = mpsc::channel();
        let value = String::from("decoded sector");
        crate::defer_log!(target: "wow::mpq", Level::Info, "owned value: {}", {
            assert_ne!(thread::current().id(), caller);
            sender.send(thread::current().id()).expect("format observer");
            value
        });
        let formatter = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("worker format");
        crate::defer_log!(target: "wow::other", Level::Info, "filtered record {}", disabled_argument());
        defer(Level::Info, "wow::other", || panic!("disabled closure ran"));
        log::info!(target: "wow::mpq", "compatibility record");
        let (sender, receiver) = mpsc::channel();
        defer(Level::Info, "wow::mpq", move || {
            let here = thread::current().id();
            defer(Level::Info, "wow::mpq", move || {
                assert_eq!(thread::current().id(), here);
                sender.send(here).expect("queue barrier");
            });
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            formatter
        );
        log::logger().flush();
    }

    #[test]
    fn concurrent_producers_move_owned_jobs_in_order() {
        let (sender, receiver) = channel();
        let (done, results) = channel();
        let consumer = thread::spawn(move || {
            for message in receiver {
                // Deferred jobs do not need a backend; their captures are owned.
                Message::emit(message, &log::logger());
            }
        });
        let producers: Vec<_> = (0..4)
            .map(|producer| {
                let sender = sender.clone();
                let done = done.clone();
                thread::spawn(move || {
                    for sequence in 0..100 {
                        let done = done.clone();
                        let value = vec![producer, sequence];
                        sender
                            .send(Message::Deferred(Box::new(move || {
                                done.send(value).expect("result consumer");
                            })))
                            .expect("worker receiver");
                    }
                })
            })
            .collect();
        drop(sender);
        drop(done);
        for producer in producers {
            producer.join().unwrap();
        }
        consumer.join().unwrap();
        let mut next = [0; 4];
        for result in results {
            assert_eq!(result[1], next[result[0]]);
            next[result[0]] += 1;
        }
        assert_eq!(next, [100; 4]);
    }
}
