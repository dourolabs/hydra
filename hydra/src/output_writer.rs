//! Stdout/stderr write helpers that tag BrokenPipe errors with a sentinel.
//!
//! Background: writing to stdout/stderr can fail with `ErrorKind::BrokenPipe` when
//! the reader closes the pipe early (e.g. `hydra issues list | head -5`). We want
//! to treat that as a clean exit, not a CLI error. But other parts of the call
//! graph (reqwest's hyper transport, git subprocesses) can also surface
//! BrokenPipe deep in an error chain, and those must NOT be silently swallowed.
//!
//! These helpers convert BrokenPipe errors at the stdout/stderr boundary into a
//! distinct `StdoutBrokenPipe` sentinel, so [`crate::cli::is_broken_pipe`] can
//! identify pipe-close-on-stdout specifically and ignore unrelated BrokenPipe
//! errors that originate elsewhere.

use std::io::{ErrorKind, StderrLock, StdoutLock, Write};

use anyhow::Result;

/// Sentinel error used to mark a BrokenPipe that originated from writing to
/// stdout or stderr (as opposed to a network or subprocess pipe).
#[derive(Debug)]
pub struct StdoutBrokenPipe;

impl std::fmt::Display for StdoutBrokenPipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("broken pipe writing to stdout/stderr")
    }
}

impl std::error::Error for StdoutBrokenPipe {}

/// Run `f` against a locked stdout writer, converting any `BrokenPipe` IO error
/// into a [`StdoutBrokenPipe`] sentinel so the top-level CLI can exit cleanly.
pub fn with_stdout<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&mut StdoutLock<'_>) -> std::io::Result<R>,
{
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    convert_pipe_error(f(&mut lock))
}

/// Run `f` against a locked stderr writer, converting any `BrokenPipe` IO error
/// into a [`StdoutBrokenPipe`] sentinel so the top-level CLI can exit cleanly.
pub fn with_stderr<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&mut StderrLock<'_>) -> std::io::Result<R>,
{
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    convert_pipe_error(f(&mut lock))
}

/// Write `buffer` to stdout and flush; tag BrokenPipe via [`StdoutBrokenPipe`].
pub fn write_stdout(buffer: &[u8]) -> Result<()> {
    with_stdout(|w| {
        w.write_all(buffer)?;
        w.flush()
    })
}

fn convert_pipe_error<R>(result: std::io::Result<R>) -> Result<R> {
    match result {
        Ok(value) => Ok(value),
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Err(StdoutBrokenPipe.into()),
        Err(err) => Err(anyhow::Error::from(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error as IoError, ErrorKind};

    #[test]
    fn convert_pipe_error_tags_broken_pipe() {
        let err: std::io::Result<()> = Err(IoError::new(ErrorKind::BrokenPipe, "pipe gone"));
        let result = convert_pipe_error(err).unwrap_err();
        assert!(
            result
                .chain()
                .any(|c| c.downcast_ref::<StdoutBrokenPipe>().is_some()),
            "BrokenPipe IO error should be tagged with StdoutBrokenPipe sentinel"
        );
    }

    #[test]
    fn convert_pipe_error_passes_through_other_errors() {
        let err: std::io::Result<()> = Err(IoError::new(ErrorKind::NotFound, "missing"));
        let result = convert_pipe_error(err).unwrap_err();
        assert!(
            !result
                .chain()
                .any(|c| c.downcast_ref::<StdoutBrokenPipe>().is_some()),
            "non-BrokenPipe errors must not be tagged with the sentinel"
        );
    }

    #[test]
    fn convert_pipe_error_ok_passes_through() {
        let ok: std::io::Result<i32> = Ok(42);
        assert_eq!(convert_pipe_error(ok).unwrap(), 42);
    }
}
