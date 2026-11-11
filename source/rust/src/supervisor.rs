use crate::platform::{Job, Process};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::mpsc::{Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::PostMessageW};
pub const STOPPED: u32 = 0x8001;
pub const FAILED: u32 = 0x8002;
pub enum Action {
    Restart,
    Exit,
}

#[derive(Default)]
pub struct RetryBudget {
    failures: VecDeque<Instant>,
}
impl RetryBudget {
    pub fn take(&mut self, now: Instant) -> bool {
        while self
            .failures
            .front()
            .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
        {
            self.failures.pop_front();
        }
        if self.failures.len() >= 2 {
            return false;
        }
        self.failures.push_back(now);
        true
    }
    fn clear(&mut self) {
        self.failures.clear();
    }
}
struct Engine {
    child: Process,
    job: Job,
    port: u16,
}
impl Engine {
    fn start(root: &std::path::Path) -> io::Result<Self> {
        let job = Job::new()?;
        let port = available_port(7000)?;
        let child = job.spawn(&std::env::current_exe()?, root, port)?;
        log::info!("started Rust receiver pid={} port={port}", child.id());
        Ok(Self { child, job, port })
    }
    fn stop(&mut self) {
        crate::platform::close_receiver(self.child.id());
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(40));
        }
        log::warn!("receiver shutdown timed out; terminating job");
        self.job.terminate();
        let _ = self.child.wait();
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.job.terminate();
        let _ = self.child.wait();
    }
}

pub fn worker(root: PathBuf, actions: Receiver<Action>, hwnd: usize) {
    let notify = |msg| unsafe {
        PostMessageW(hwnd as HWND, msg, 0, 0);
    };
    let mut budget = RetryBudget::default();
    loop {
        let mut engine = match Engine::start(&root) {
            Ok(e) => e,
            Err(e) => {
                log::error!("receiver start failed: {e}");
                notify(FAILED);
                match actions.recv() {
                    Ok(Action::Restart) => {
                        budget.clear();
                        continue;
                    }
                    _ => {
                        notify(STOPPED);
                        return;
                    }
                }
            }
        };
        let started = Instant::now();
        let mut last_probe = Instant::now();
        let mut failures = 0;
        let automatic = loop {
            match actions.recv_timeout(Duration::from_millis(200)) {
                Ok(Action::Exit) | Err(RecvTimeoutError::Disconnected) => {
                    engine.stop();
                    notify(STOPPED);
                    return;
                }
                Ok(Action::Restart) => {
                    budget.clear();
                    break false;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            match engine.child.try_wait() {
                Ok(Some(status)) => {
                    log::info!("receiver exited code={status}");
                    if status == 0 {
                        notify(STOPPED);
                        return;
                    }
                    if status == 2 {
                        budget.clear();
                        break false;
                    }
                    break true;
                }
                Err(e) => {
                    log::error!("cannot monitor receiver: {e}");
                    break true;
                }
                Ok(None) => {}
            }
            if started.elapsed() >= Duration::from_secs(12)
                && last_probe.elapsed() >= Duration::from_secs(5)
            {
                last_probe = Instant::now();
                if probe(engine.port).is_ok() {
                    failures = 0;
                } else {
                    failures += 1;
                    log::warn!("control probe failed count={failures}");
                }
                if failures >= 3 {
                    log::warn!("receiver control server unresponsive");
                    break true;
                }
            }
        };
        engine.stop();
        drop(engine);
        if automatic && !budget.take(Instant::now()) {
            log::error!("automatic recovery limit reached");
            notify(FAILED);
            match actions.recv() {
                Ok(Action::Restart) => budget.clear(),
                _ => {
                    notify(STOPPED);
                    return;
                }
            }
        }
        // Do not spin or freeze the tray while waiting for the old sockets to close.
        match actions.recv_timeout(Duration::from_millis(500)) {
            Ok(Action::Exit) | Err(RecvTimeoutError::Disconnected) => {
                notify(STOPPED);
                return;
            }
            Ok(Action::Restart) => budget.clear(),
            _ => {}
        }
    }
}

fn available_port(preferred: u16) -> io::Result<u16> {
    let listener = match TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, preferred)) {
        Ok(listener) => listener,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
            ) =>
        {
            let listener = TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, 0))?;
            log::warn!(
                "control port {preferred} unavailable; choosing {}",
                listener.local_addr()?.port()
            );
            listener
        }
        Err(error) => return Err(error),
    };
    // Released just before spawning. A concurrent bind race is retried through the
    // normal bounded recovery path; the other application's listener is untouched.
    Ok(listener.local_addr()?.port())
}

pub fn probe(port: u16) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(1800);
    let mut socket = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(500),
    )?;
    socket.set_write_timeout(Some(Duration::from_millis(400)))?;
    socket.write_all(b"GET /info RTSP/1.0\r\nCSeq: 1\r\n\r\n")?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192 {
            return Err(io::Error::other("Oversized RTSP header"));
        }
        socket.set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ))?;
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Probe deadline exceeded",
            ));
        }
        let mut byte = [0];
        socket.read_exact(&mut byte)?;
        header.push(byte[0]);
    }
    let header = std::str::from_utf8(&header).map_err(io::Error::other)?;
    if !header.starts_with("RTSP/1.0 200 ") && !header.starts_with("HTTP/1.1 200 ") {
        return Err(io::Error::other("Unexpected RTSP status"));
    }
    let mut length = None;
    for line in header.lines() {
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("Content-Length")
        {
            if length.is_some() {
                return Err(io::Error::other("Duplicate Content-Length"));
            }
            length = Some(value.trim().parse::<usize>().map_err(io::Error::other)?);
        }
    }
    let n = length
        .filter(|n| (8..=65536).contains(n))
        .ok_or_else(|| io::Error::other("Invalid plist length"))?;
    let mut body = vec![0; n];
    let mut offset = 0;
    while offset < n {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Probe deadline exceeded",
            ));
        }
        socket.set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ))?;
        let got = socket.read(&mut body[offset..])?;
        if got == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Truncated plist",
            ));
        }
        offset += got;
    }
    if !body.starts_with(b"bplist00") {
        return Err(io::Error::other("Not a binary plist"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn occupied_control_port_selects_another_without_displacing_owner() {
        let owner = TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let occupied = owner.local_addr().unwrap().port();
        let selected = available_port(occupied).unwrap();
        assert_ne!(selected, occupied);
        assert_ne!(selected, 0);
        let _client = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, occupied)).unwrap();
        assert!(owner.accept().is_ok());
        let probe = TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, selected)).unwrap();
        assert_eq!(probe.local_addr().unwrap().port(), selected);
    }
    #[test]
    fn bounded_retries_use_monotonic_time() {
        let mut b = RetryBudget::default();
        let now = Instant::now();
        assert!(b.take(now));
        assert!(b.take(now));
        assert!(!b.take(now));
        assert!(b.take(now + Duration::from_secs(60)));
    }
    #[test]
    fn probe_handles_fragmentation_and_rejects_ambiguous_lengths() {
        use std::net::TcpListener;
        for good in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let t = thread::spawn(move || {
                let (mut s, _) = listener.accept().unwrap();
                let mut request = [0; 128];
                let _ = s.read(&mut request);
                let response = if good {
                    b"RTSP/1.0 200 OK\r\nContent-Length: 8\r\n\r\nbplist00".as_slice()
                } else {
                    b"RTSP/1.0 200 OK\r\nContent-Length: 8\r\nContent-Length: 9\r\n\r\nbplist00"
                        .as_slice()
                };
                for byte in response {
                    if s.write_all(&[*byte]).is_err() {
                        break;
                    }
                }
            });
            assert_eq!(probe(port).is_ok(), good);
            t.join().unwrap();
        }
    }
}
