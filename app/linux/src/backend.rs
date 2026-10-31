use crate::local_api::{BackendOrigin, LocalApi};
use nix::{
    fcntl::{fcntl, FcntlArg, OFlag},
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use serde::Deserialize;
use std::{
    fs::{self, DirBuilder},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const START_ERROR: &str = "Le service local Ezerbot n'a pas pu démarrer.";
const READY_LIMIT: usize = 4096;

pub struct Backend {
    child: ChildGuard,
    pub api: Arc<LocalApi>,
}

impl Backend {
    pub fn start(
        wrapper: &Path,
        directory: &Path,
        quit: &AtomicBool,
    ) -> Result<Self, &'static str> {
        Self::start_with_timeout(wrapper, directory, quit, Duration::from_secs(240))
    }

    fn start_with_timeout(
        wrapper: &Path,
        directory: &Path,
        quit: &AtomicBool,
        timeout: Duration,
    ) -> Result<Self, &'static str> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .map_err(|_| START_ERROR)?;
        let metadata = fs::symlink_metadata(directory).map_err(|_| START_ERROR)?;
        if !metadata.is_dir()
            || metadata.uid() != nix::unistd::geteuid().as_raw()
            || metadata.mode() & 0o077 != 0
        {
            return Err("Le dossier de données Ezerbot doit être privé et vous appartenir.");
        }
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|_| START_ERROR)?;
        let spawned = Command::new(wrapper)
            .args(["--port", "0", "--db"])
            .arg(directory.join("ezerbot.sqlite"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => ChildGuard::new(child),
            Err(_) => {
                bytes.fill(0);
                return Err(START_ERROR);
            }
        };
        let identity = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let delivered = child
            .child
            .as_mut()
            .and_then(|child| child.stdin.take())
            .ok_or(START_ERROR)
            .and_then(|mut input| input.write_all(&bytes).map_err(|_| START_ERROR));
        bytes.fill(0);
        delivered?;
        let mut stdout = child
            .child
            .as_mut()
            .and_then(|child| child.stdout.take())
            .ok_or(START_ERROR)?;
        use std::os::fd::AsRawFd;
        let flags = fcntl(stdout.as_raw_fd(), FcntlArg::F_GETFL).map_err(|_| START_ERROR)?;
        fcntl(
            stdout.as_raw_fd(),
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
        )
        .map_err(|_| START_ERROR)?;
        let deadline = Instant::now() + timeout;
        let mut line = Vec::new();
        let origin = loop {
            if quit.load(Ordering::SeqCst) || Instant::now() >= deadline || child.exited() {
                return Err(START_ERROR);
            }
            let mut chunk = [0u8; 512];
            match stdout.read(&mut chunk) {
                Ok(0) => return Err(START_ERROR),
                Ok(length) => {
                    let end = chunk[..length].iter().position(|byte| *byte == b'\n');
                    line.extend_from_slice(&chunk[..end.unwrap_or(length)]);
                    if line.len() > READY_LIMIT {
                        return Err(START_ERROR);
                    }
                    if end.is_some() {
                        break parse_ready(&line)?;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return Err(START_ERROR),
            }
        };
        let stop = Arc::clone(&child.stop);
        child.drain = Some(thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while !stop.load(Ordering::Relaxed) {
                match stdout.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        }));
        Ok(Self {
            child,
            api: Arc::new(LocalApi::new(origin, identity)),
        })
    }

    pub fn shutdown(&mut self) {
        self.child.shutdown();
    }
    pub fn exited(&mut self) -> bool {
        self.child.exited()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    url: String,
}

fn parse_ready(line: &[u8]) -> Result<BackendOrigin, &'static str> {
    let ready: Ready = serde_json::from_slice(line).map_err(|_| START_ERROR)?;
    BackendOrigin::parse(&ready.url).ok_or(START_ERROR)
}

struct ChildGuard {
    child: Option<Child>,
    stop: Arc<AtomicBool>,
    drain: Option<JoinHandle<()>>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self {
            child: Some(child),
            stop: Arc::new(AtomicBool::new(false)),
            drain: None,
        }
    }

    fn exited(&mut self) -> bool {
        self.child
            .as_mut()
            .is_none_or(|child| !matches!(child.try_wait(), Ok(None)))
    }

    fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM);
                let deadline = Instant::now() + Duration::from_millis(1250);
                while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(20));
                }
                if !matches!(child.try_wait(), Ok(Some(_))) {
                    let _ = child.kill();
                }
            }
            let _ = child.wait();
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, path::PathBuf, sync::atomic::AtomicU64};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new(body: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "ezerbot-linux-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            let wrapper = directory.join("wrapper");
            fs::write(&wrapper, format!("#!/usr/bin/python3\nimport os,sys,time,signal\nroot=os.path.dirname(__file__)\nopen(root+'/pid','w').write(str(os.getpid()))\n{}\n", body)).unwrap();
            fs::set_permissions(wrapper, fs::Permissions::from_mode(0o700)).unwrap();
            Self(directory)
        }
        fn start(&self) -> Result<Backend, &'static str> {
            Backend::start_with_timeout(
                &self.0.join("wrapper"),
                &self.0.join("state"),
                &AtomicBool::new(false),
                Duration::from_millis(300),
            )
        }
        fn assert_reaped(&self) {
            let pid: i32 = fs::read_to_string(self.0.join("pid"))
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(
                kill(Pid::from_raw(pid), None),
                Err(nix::errno::Errno::ESRCH),
                "wrapper survived cleanup"
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn delivers_private_identity_and_reaps_on_close() {
        let fixture = Fixture::new("data=sys.stdin.buffer.read()\nassert len(data)==32\nopen(root+'/delivered','w').write('32 EOF')\nprint('{\"url\":\"http://127.0.0.1:32123\"}',flush=True)\nwhile True: time.sleep(.01)");
        let mut backend = fixture.start().unwrap();
        assert_eq!(
            fs::read_to_string(fixture.0.join("delivered")).unwrap(),
            "32 EOF"
        );
        assert_eq!(
            fs::metadata(fixture.0.join("state"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        backend.shutdown();
        backend.shutdown();
        fixture.assert_reaped();
    }

    #[test]
    fn startup_failures_reap_the_child() {
        for output in [
            "print('not-json',flush=True)",
            "print('x'*5000,flush=True)",
            "sys.stdout.close()",
            "time.sleep(2)",
            "sys.exit(0)",
        ] {
            let fixture =
                Fixture::new(&format!("sys.stdin.buffer.read()\n{output}\ntime.sleep(2)"));
            assert!(fixture.start().is_err());
            fixture.assert_reaped();
        }
    }

    #[test]
    fn forces_shutdown_when_wrapper_ignores_term() {
        let fixture = Fixture::new("sys.stdin.buffer.read()\nsignal.signal(signal.SIGTERM,signal.SIG_IGN)\nprint('{\"url\":\"http://127.0.0.1:32123\"}',flush=True)\nwhile True: time.sleep(.01)");
        let backend = fixture.start().unwrap();
        let start = Instant::now();
        drop(backend);
        assert!(start.elapsed() < Duration::from_secs(3));
        fixture.assert_reaped();
    }

    #[test]
    fn rejects_shared_and_symlink_data_directories_without_changing_them() {
        let fixture = Fixture::new("sys.exit(99)");
        let shared = fixture.0.join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            Backend::start(&fixture.0.join("wrapper"), &shared, &AtomicBool::new(false)).is_err()
        );
        assert_eq!(
            fs::metadata(&shared).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let link = fixture.0.join("link");
        std::os::unix::fs::symlink(&shared, &link).unwrap();
        assert!(
            Backend::start(&fixture.0.join("wrapper"), &link, &AtomicBool::new(false)).is_err()
        );
        assert!(!fixture.0.join("pid").exists());
    }

    #[test]
    fn quit_during_startup_reaps_the_child() {
        let fixture = Fixture::new("sys.stdin.buffer.read()\ntime.sleep(2)");
        let quit = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(100));
                quit.store(true, Ordering::SeqCst);
            });
            assert!(
                Backend::start(&fixture.0.join("wrapper"), &fixture.0.join("state"), &quit)
                    .is_err()
            );
        });
        fixture.assert_reaped();
    }

    #[test]
    fn readiness_rejects_extra_fields_and_untrusted_origins() {
        assert!(parse_ready(br#"{"url":"http://127.0.0.1:32123"}"#).is_ok());
        for line in [
            br#"{"url":"http://localhost:32123"}"#.as_slice(),
            br#"{"url":"http://127.0.0.1:32123","extra":true}"#,
            br#"{"url":12}"#,
        ] {
            assert!(parse_ready(line).is_err());
        }
    }
}
