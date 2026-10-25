//! Starts the command-line tools that captures use, such as adb and
//! Xcode's tools, and runs a command to its end within a time limit.
use std::{
    io::{self, Read},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// A command without input, so a tool that asks something fails instead
/// of waiting. On Windows it opens no console window.
pub(super) fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}
pub(super) struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}
/// Reads a pipe to its end on its own thread, keeping `limit` bytes.
fn drain(pipe: Option<impl Read + Send + 'static>, limit: u64) -> mpsc::Receiver<Vec<u8>> {
    let (sent, received) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        thread::spawn(move || {
            let mut out = Vec::new();
            let _ = pipe.by_ref().take(limit).read_to_end(&mut out);
            // The rest is read too, so the program never waits on the pipe.
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            let _ = sent.send(out);
        });
    }
    received
}
/// Runs `command` to its end within `timeout`; past it, the error's kind is
/// `TimedOut`. A process it started, such as an adb server, may hold the
/// outputs open, so they are read only until the deadline.
pub(super) fn run(mut command: Command, timeout: Duration, limit: u64) -> io::Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = drain(child.stdout.take(), limit);
    let stderr = drain(child.stderr.take(), limit);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(io::ErrorKind::TimedOut, "command timed out"));
            }
        }
    };
    // A program that exited has written all its output, so the rest arrives
    // at once. A pipe that a child it left running holds open gets a moment,
    // not the rest of the timeout.
    let left = || {
        deadline
            .saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(100), Duration::from_secs(1))
    };
    Ok(Output {
        success: status.success(),
        stdout: stdout.recv_timeout(left()).unwrap_or_default(),
        stderr: String::from_utf8_lossy(&stderr.recv_timeout(left()).unwrap_or_default())
            .into_owned(),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut c = command(Path::new("/bin/sh"));
        c.args(["-c", script]);
        c
    }
    #[test]
    fn output_is_kept_up_to_the_limit() {
        let out = run(
            sh("printf abcdef; printf oopsie >&2; exit 3"),
            Duration::from_secs(5),
            4,
        )
        .unwrap();
        assert!(!out.success);
        assert_eq!(out.stdout, b"abcd");
        assert_eq!(out.stderr, "oops");
    }
    #[test]
    fn input_is_the_null_device() {
        // `cat` would also pass when the runner's own input is at its end.
        // This misses a lost null input only when the runner's input is the
        // null device itself.
        let out = run(
            sh("[ /dev/stdin -ef /dev/null ]"),
            Duration::from_secs(5),
            64,
        )
        .unwrap();
        assert!(out.success);
    }
    #[test]
    fn a_slow_command_times_out() {
        let e = run(sh("sleep 5"), Duration::from_millis(100), 64)
            .err()
            .expect("sleep ended within 100 ms");
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
    }
}
