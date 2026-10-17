//! Logging setup for terminal output and optional JSON-lines log files.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Log, Metadata, Record};

use crate::{ForgeError, Result};

const REDACTED: &str = "[redacted]";

fn redact_log_message(message: &str) -> String {
    let bytes = message.as_bytes();
    let mut redacted = String::with_capacity(message.len());
    let mut index = 0;

    while index < bytes.len() {
        if is_secret_key_at(bytes, index) {
            redacted.push_str(REDACTED);
            index += 56;
        } else if starts_with_ascii_case_insensitive(bytes, index, b"secret") {
            redacted.push_str(REDACTED);
            index += b"secret".len();
        } else if starts_with_ascii_case_insensitive(bytes, index, b"seed") {
            redacted.push_str(REDACTED);
            index += b"seed".len();
        } else {
            let character = message[index..].chars().next().unwrap();
            redacted.push(character);
            index += character.len_utf8();
        }
    }

    redacted
}

fn starts_with_ascii_case_insensitive(bytes: &[u8], index: usize, needle: &[u8]) -> bool {
    bytes
        .get(index..index + needle.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(needle))
}

fn is_secret_key_at(bytes: &[u8], index: usize) -> bool {
    let Some(key) = bytes.get(index..index + 56) else {
        return false;
    };

    key[0] == b'S'
        && key[1..].iter().all(|byte| is_base32_byte(*byte))
        && (index == 0 || !is_base32_byte(bytes[index - 1]))
        && (index + 56 == bytes.len() || !is_base32_byte(bytes[index + 56]))
}

fn is_base32_byte(byte: u8) -> bool {
    byte.is_ascii_uppercase() || (b'2'..=b'7').contains(&byte)
}

#[cfg(test)]
mod tests {
    use super::redact_log_message;

    #[test]
    fn redacts_secret_key_values() {
        let secret_key = format!("S{}", "A".repeat(55));
        let message = format!("signing key: {secret_key}");

        assert_eq!(redact_log_message(&message), "signing key: [redacted]");
    }

    #[test]
    fn redacts_labels_without_case_sensitivity() {
        assert_eq!(
            redact_log_message("Secret SECRET sEcReT; Seed SEED sEeD"),
            "[redacted] [redacted] [redacted]; [redacted] [redacted] [redacted]"
        );
    }
}

struct ForgeLogger {
    console: env_logger::Logger,
    file: Option<Mutex<File>>,
    file_level: log::LevelFilter,
}

impl Log for ForgeLogger {
    /// Enables records accepted by either the console or the file's level filter.
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.console.enabled(metadata)
            || (self.file.is_some() && metadata.level().to_level_filter() <= self.file_level)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        if self.console.enabled(record.metadata()) {
            self.console.log(record);
        }

        if let Some(file) = &self.file {
            let timestamp_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let entry = serde_json::json!({
                "timestamp_ms": timestamp_ms,
                "level": record.level().as_str(),
                "target": record.target(),
                "message": redact_log_message(&record.args().to_string()),
            });
            if let Ok(mut file) = file.lock() {
                let _ = serde_json::to_writer(&mut *file, &entry);
                let _ = file.write_all(b"\n");
                let _ = file.flush();
            }
        }
    }

    fn flush(&self) {
        self.console.flush();
        if let Some(file) = &self.file {
            if let Ok(mut file) = file.lock() {
                let _ = file.flush();
            }
        }
    }
}

/// Initialize normal stderr logging and, when requested, a JSON-lines log.
///
/// `verbose` is the number of times `-v`/`--verbose` was passed: `0` keeps
/// the default `info` level, `1` (`-v`) raises it to `debug`, and `2+`
/// (`-vv`) raises it to `trace`.
pub fn init(verbose: u8, log_file: Option<&Path>) -> Result<()> {
    init_with_color(verbose, log_file, true)
}

/// Like [`init`], but with explicit color control.
///
/// When `use_color` is `false` the console logger writes plain text with no
/// ANSI escape sequences, honoring `NO_COLOR` / `--color never`.
pub fn init_with_color(verbose: u8, log_file: Option<&Path>, use_color: bool) -> Result<()> {
    let level = verbosity_level(verbose);
    let mut builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(level.to_string()),
    );
    builder.format_timestamp(None);
    if !use_color {
        builder.write_style(env_logger::WriteStyle::Never);
    }
    let console = builder.build();
    let max_level = console.filter();

    let file = log_file
        .map(|path| {
            File::create(path)
                .map(Mutex::new)
                .map_err(ForgeError::io(format!(
                    "creating log file {}",
                    path.display()
                )))
        })
        .transpose()?;

    let logger = ForgeLogger {
        console,
        file,
        file_level: max_level,
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(max_level);
    }
    Ok(())
}

fn verbosity_level(verbose: u8) -> log::LevelFilter {
    match verbose {
        0 => log::LevelFilter::Info,
        1 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    }
}

#[cfg(test)]
mod logging_tests {
    use std::fs::File;
    use std::sync::Mutex;

    use log::{Log, Record};

    use super::{verbosity_level, ForgeLogger};

    #[test]
    fn trace_records_are_written_to_file_at_verbosity_two() {
        let verbosity = 2;
        let file_level = verbosity_level(verbosity);
        let console = env_logger::Builder::new()
            .filter_level(file_level)
            .build();
        let log_file = tempfile::NamedTempFile::new().unwrap();
        let logger = ForgeLogger {
            console,
            file: Some(Mutex::new(File::create(log_file.path()).unwrap())),
            file_level,
        };
        let message = "trace diagnostic";
        let args = format_args!("{message}");
        let record = Record::builder()
            .level(log::Level::Trace)
            .target("logging_test")
            .args(args)
            .build();

        logger.log(&record);
        logger.flush();

        let contents = std::fs::read_to_string(log_file.path()).unwrap();
        let entry: serde_json::Value = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(entry["level"], "TRACE");
        assert_eq!(entry["message"], message);
    }
}
