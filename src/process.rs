use crate::control::Control;
use anyhow::{Context, Result, bail};
use std::{
    io::{Read, Write},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
pub fn run(command: &mut Command, timeout: Duration, control: &Control) -> Result<Output> {
    run_with_input(command, timeout, control, None)
}
pub fn run_with_input(
    command: &mut Command,
    timeout: Duration,
    control: &Control,
    input: Option<&[u8]>,
) -> Result<Output> {
    control.check()?;
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().context("failed to start command")?;
    let mut stdout = child.stdout.take().context("missing stdout pipe")?;
    let mut stderr = child.stderr.take().context("missing stderr pipe")?;
    let (send_out, receive_out) = std::sync::mpsc::channel();
    let (send_err, receive_err) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut data = Vec::new();
        let result = stdout
            .by_ref()
            .take(16 * 1024 * 1024)
            .read_to_end(&mut data)
            .map(|_| data);
        let _ = send_out.send(result);
    });
    thread::spawn(move || {
        let mut data = Vec::new();
        let result = stderr
            .by_ref()
            .take(16 * 1024 * 1024)
            .read_to_end(&mut data)
            .map(|_| data);
        let _ = send_err.send(result);
    });
    if let Some(input) = input
        && let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(input)
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.into());
    }
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if let Err(error) = control.check() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("command timed out after {} seconds", timeout.as_secs());
        }
        thread::sleep(Duration::from_millis(50));
    };
    // Descendants can retain inherited pipes: never block the GUI indefinitely.
    Ok(Output {
        status,
        stdout: receive_out
            .recv_timeout(Duration::from_secs(1))
            .context("stdout pipe did not close")??,
        stderr: receive_err
            .recv_timeout(Duration::from_secs(1))
            .context("stderr pipe did not close")??,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sleeper() -> Command {
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 5"]);
            command
        }
        #[cfg(windows)]
        {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 5",
            ]);
            command
        }
    }
    #[test]
    fn cancellation_returns_without_waiting_for_command_or_inherited_pipes() {
        let control = Control::default();
        let child_control = control.clone();
        let started = Instant::now();
        let handle =
            thread::spawn(move || run(&mut sleeper(), Duration::from_secs(10), &child_control));
        thread::sleep(Duration::from_millis(100));
        control.cancel();
        assert!(handle.join().unwrap().is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn independent_timeout_bounds_a_slow_command() {
        let started = Instant::now();
        assert!(
            run(
                &mut sleeper(),
                Duration::from_millis(100),
                &Control::default()
            )
            .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
