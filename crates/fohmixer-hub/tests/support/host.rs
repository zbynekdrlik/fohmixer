//! `sim/host.py` processes (Unix: stopped with `kill -s TERM`).

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fohmixer_hub::config::InstanceCfg;

use super::repo;

/// A `sim/host.py` process.
pub struct Host {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    pub port: u16,
    pub instance: String,
    _logs: tempfile::TempDir,
}

impl Host {
    /// A host of `instance` on a port of its own.
    pub fn start(instance: &str) -> Self {
        Self::start_with(instance, 0, 0.0)
    }

    /// A host on `port` (0: the OS picks), its meters moving `meters_hz`
    /// times a second (0: still).
    pub fn start_with(instance: &str, port: u16, meters_hz: f64) -> Self {
        let site = repo().join("sim").join("fixtures").join("test-site.json");
        Self::start_site(instance, &site, port, meters_hz)
    }

    /// A host of the set `site` (a SimLive site fixture), otherwise as
    /// [`Host::start_with`].
    pub fn start_site(instance: &str, site: &Path, port: u16, meters_hz: f64) -> Self {
        let python = std::env::var("FOHMIXER_PYTHON").unwrap_or_else(|_| "python3".to_string());
        let logs = tempfile::tempdir().unwrap();
        let root = repo();
        let mut child = Command::new(python)
            .arg(root.join("sim").join("host.py"))
            .args(["--port", &port.to_string(), "--instance", instance])
            .arg("--site")
            .arg(site)
            .args(["--meters-hz", &meters_hz.to_string()])
            .arg("--log-dir")
            .arg(logs.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn python3 sim/host.py");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let ready = lines
            .recv_timeout(Duration::from_secs(15))
            .expect("the host prints READY");
        let port = ready
            .strip_prefix("READY ")
            .and_then(|p| p.trim().parse().ok())
            .unwrap_or_else(|| panic!("not a READY line: {ready}"));
        Self {
            child,
            stdin,
            lines,
            port,
            instance: instance.to_string(),
            _logs: logs,
        }
    }

    /// The hub's config entry of this host.
    pub fn cfg(&self) -> InstanceCfg {
        InstanceCfg {
            name: self.instance.clone(),
            port: self.port,
        }
    }

    fn line(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Blocks the host's Live main thread for `ms`.
    pub fn stall(&mut self, ms: u64) {
        self.line(&format!("stall {ms}"));
    }

    fn answer(&mut self, line: &str) -> String {
        self.line(line);
        self.lines
            .recv_timeout(Duration::from_secs(5))
            .expect("the host answers")
    }

    /// Renames every track named `old` (Live's listeners fire); the count.
    pub fn rename(&mut self, old: &str, new: &str) -> usize {
        let answer = self.answer(&format!("rename \"{old}\" \"{new}\""));
        answer
            .strip_prefix("RENAMED ")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("not a RENAMED line: {answer}"))
    }

    /// Sets (`set`), adds (`add`) or removes (`remove`) a Tuner marker on the
    /// track (`kind` `track` or `return`) at `index` (#68); the Tuners it
    /// then holds (-1: no such track).
    pub fn tuner(&mut self, kind: &str, index: u32, action: &str, name: &str) -> i64 {
        let quoted = name.replace('\'', "'\"'\"'");
        let line = if action == "remove" {
            format!("tuner {kind} {index} remove")
        } else {
            format!("tuner {kind} {index} {action} '{quoted}'")
        };
        let answer = self.answer(&line);
        answer
            .strip_prefix("TUNER ")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("not a TUNER line: {answer}"))
    }

    /// Deletes the track at `index` as a user in Live would; the tracks left
    /// (-1: no such track).
    pub fn delete_track(&mut self, index: u32) -> i64 {
        let answer = self.answer(&format!("delete-track {index}"));
        answer
            .strip_prefix("DELETED ")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("not a DELETED line: {answer}"))
    }

    /// The Live listeners on `prop` of the object at `path` (-1: none there).
    pub fn listeners(&mut self, prop: &str, path: &str) -> i64 {
        let answer = self.answer(&format!("listeners {prop} {path}"));
        answer
            .strip_prefix("LISTENERS ")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("not a LISTENERS line: {answer}"))
    }

    /// Asks the host to stop (SIGTERM) and waits for it (bounded).
    pub fn stop(mut self) -> ExitStatus {
        self.request_stop()
    }

    fn request_stop(&mut self) -> ExitStatus {
        if let Ok(Some(status)) = self.child.try_wait() {
            return status;
        }
        let sent = Command::new("kill")
            .args(["-s", "TERM", &self.child.id().to_string()])
            .status()
            .expect("run kill");
        assert!(sent.success(), "SIGTERM not sent");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the host did not stop within 10 s"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = Command::new("kill")
                .args(["-s", "TERM", &self.child.id().to_string()])
                .status();
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.child.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}
