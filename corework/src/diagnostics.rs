//! Low-overhead diagnostics shared by runtime-facing crates.
//!
//! Producers never perform file IO on an async execution path. Messages are
//! sent to a bounded worker queue and dropped when the queue is saturated.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const DIAGNOSTIC_QUEUE_CAPACITY: usize = 4096;

static DIAGNOSTIC_SENDER: OnceLock<SyncSender<String>> = OnceLock::new();
static DROPPED_MESSAGES: AtomicU64 = AtomicU64::new(0);

pub fn set_log_file_path(path: impl AsRef<Path>) {
    let path = path.as_ref().to_path_buf();
    let _ = DIAGNOSTIC_SENDER.get_or_init(|| start_writer(path));
}

pub fn append_line(line: impl AsRef<str>) {
    let Some(sender) = DIAGNOSTIC_SENDER.get() else {
        return;
    };
    let dropped = DROPPED_MESSAGES.swap(0, Ordering::Relaxed);
    let line = if dropped == 0 {
        line.as_ref().to_string()
    } else {
        format!(
            "[diagnostics] dropped_messages={} queue_capacity={}\n{}",
            dropped,
            DIAGNOSTIC_QUEUE_CAPACITY,
            line.as_ref()
        )
    };
    match sender.try_send(line) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            DROPPED_MESSAGES.fetch_add(dropped.saturating_add(1), Ordering::Relaxed);
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
}

fn start_writer(path: PathBuf) -> SyncSender<String> {
    let (sender, receiver) = mpsc::sync_channel::<String>(DIAGNOSTIC_QUEUE_CAPACITY);
    let _ = std::thread::Builder::new()
        .name("corework-diagnostics".to_string())
        .spawn(move || {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
                return;
            };
            while let Ok(line) = receiver.recv() {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or_default();
                let _ = writeln!(file, "[{timestamp}] {line}");
            }
        });
    sender
}
