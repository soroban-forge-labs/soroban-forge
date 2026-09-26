//! Support for the global `--timeout` flag.
//!
//! Network-capable operations bound their work with the value from
//! [`ForgeContext::timeout`](crate::ForgeContext::timeout). HTTP calls apply it
//! directly (`ureq`'s `.timeout(..)`); subprocess calls that can hang on
//! network I/O go through [`output_with_timeout`] / [`status_with_timeout`],
//! which kill the child once the deadline passes.

use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::{ForgeError, Result};

/// How often a running child is polled while waiting for the deadline.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Parse the value of `--timeout`: a whole number of seconds greater than zero.
pub fn parse_timeout_secs(value: &str) -> Result<u64> {
    match value.trim().parse::<u64>() {
        Ok(0) => Err(ForgeError::InvalidArgument(
            "--timeout must be greater than 0 seconds".into(),
        )),
        Ok(secs) => Ok(secs),
        Err(_) => Err(ForgeError::InvalidArgument(format!(
            "--timeout `{value}` is not a valid number of seconds"
        ))),
    }
}

fn timed_out(timeout: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("timed out after {timeout:?} (--timeout)"),
    )
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    })
}

/// Wait for `child` to exit, killing it and returning a `TimedOut` error if it
/// is still running once `timeout` has elapsed.
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> io::Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(timed_out(timeout));
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline - now));
    }
}

/// Like [`Command::output`], but gives up after `timeout` (when set).
///
/// On timeout the child is killed and an [`io::ErrorKind::TimedOut`] error is
/// returned. With `timeout == None` this is exactly `cmd.output()`.
pub fn output_with_timeout(cmd: &mut Command, timeout: Option<Duration>) -> io::Result<Output> {
    let Some(timeout) = timeout else {
        return cmd.output();
    };

    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let status = wait_with_timeout(&mut child, timeout)?;
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Like [`Command::status`], but gives up after `timeout` (when set).
///
/// On timeout the child is killed and an [`io::ErrorKind::TimedOut`] error is
/// returned. With `timeout == None` this is exactly `cmd.status()`.
pub fn status_with_timeout(cmd: &mut Command, timeout: Option<Duration>) -> io::Result<ExitStatus> {
    let Some(timeout) = timeout else {
        return cmd.status();
    };
    let mut child = cmd.spawn()?;
    wait_with_timeout(&mut child, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_positive_seconds() {
        assert_eq!(parse_timeout_secs("30").unwrap(), 30);
    }

    #[test]
    fn parse_rejects_zero_with_clear_error() {
        let err = parse_timeout_secs("0").unwrap_err().to_string();
        assert!(err.contains("--timeout") && err.contains("greater than 0"), "{err}");
    }

    #[test]
    fn parse_rejects_non_numeric_with_clear_error() {
        for bad in ["abc", "-5", "1.5", ""] {
            let err = parse_timeout_secs(bad).unwrap_err().to_string();
            assert!(err.contains("--timeout") && err.contains("not a valid number"), "{bad}: {err}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn slow_subprocess_is_bounded_by_timeout() {
        let started = Instant::now();
        let err = output_with_timeout(
            Command::new("sleep").arg("10"),
            Some(Duration::from_millis(200)),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn slow_subprocess_status_is_bounded_by_timeout() {
        let started = Instant::now();
        let err = status_with_timeout(
            Command::new("sleep").arg("10"),
            Some(Duration::from_millis(200)),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn fast_subprocess_output_is_captured_within_timeout() {
        let out = output_with_timeout(
            Command::new("echo").arg("hello"),
            Some(Duration::from_secs(10)),
        )
        .unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }

    #[cfg(unix)]
    #[test]
    fn no_timeout_behaves_like_plain_output() {
        let out = output_with_timeout(Command::new("echo").arg("hi"), None).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }
}
