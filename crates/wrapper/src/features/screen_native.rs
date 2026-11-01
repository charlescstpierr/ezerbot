//! Release-only transport to the native helper shipped beside the wrapper. No
//! executable override, socket discovery, shell, or test environment switch can
//! reach it.
use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Instant,
};
const UNPROVEN: &str = "Partage en pause : bandeau non garanti visible.";
fn executable() -> Result<PathBuf, ApiError> {
    Ok(std::env::current_exe()
        .map_err(|_| unavailable())?
        .parent()
        .ok_or_else(unavailable)?
        .join(if cfg!(target_os = "linux") {
            "ezerbot-linux"
        } else {
            "Ezerbot"
        }))
}
fn unavailable() -> ApiError {
    ApiError::new(503,"Le helper natif est indisponible. Ouvrez l’application Ezerbot construite, puis réessayez.")
}
fn error(v: &Value) -> ApiError {
    ApiError::new(
        403,
        v["error"]
            .as_str()
            .unwrap_or("L’action sur l’écran a échoué."),
    )
}
pub fn permissions() -> Result<(), ApiError> {
    let status = Command::new(executable()?)
        .arg("--screen-permissions")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| unavailable())?;
    if !status.success() {
        #[cfg(target_os = "linux")]
        return Err(ApiError::new(403,"Aucune capture possible dans cette session. Le partage d’écran fonctionne sous X11 sans compositeur."));
        #[cfg(not(target_os = "linux"))]
        return Err(ApiError::new(403,"Autorisez Accessibilité, Enregistrement de l’écran et Surveillance de l’entrée dans Réglages Système, puis réactivez ce bot."));
    }
    Ok(())
}
pub struct Client {
    input: Mutex<ChildStdin>,
    child: Mutex<Child>,
    replies: Mutex<Receiver<Value>>,
    paused: Arc<AtomicBool>,
    unproven: Arc<AtomicBool>,
    revoked: Arc<AtomicBool>,
    sequence: AtomicU64,
}
impl Client {
    pub fn start(
        duration: u64,
        permit: Arc<Mutex<()>>,
        generation: Arc<AtomicU64>,
    ) -> Result<Self, ApiError> {
        let mut command = Command::new(executable()?);
        command.args(["--screen-helper", &duration.to_string()]);
        Self::launch(command, permit, generation)
    }
    pub(super) fn launch(
        mut command: Command,
        permit: Arc<Mutex<()>>,
        generation: Arc<AtomicU64>,
    ) -> Result<Self, ApiError> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| unavailable())?;
        let input = child.stdin.take().ok_or_else(unavailable)?;
        let output = child.stdout.take().ok_or_else(unavailable)?;
        let paused = Arc::new(AtomicBool::new(false));
        let unproven = Arc::new(AtomicBool::new(false));
        let revoked = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (p, u, r) = (paused.clone(), unproven.clone(), revoked.clone());
        std::thread::spawn(move || {
            // The generation moves first, so a capture on its way to the COMMIT is
            // dropped. The permit then waits for a decision taken before the stop:
            // once a flag reads stopped, no capture can still be stored.
            let halt = |flag: &AtomicBool| {
                generation.fetch_add(1, Ordering::SeqCst);
                let _permit = permit.lock().unwrap_or_else(|e| e.into_inner());
                flag.store(true, Ordering::SeqCst);
            };
            let mut reader = BufReader::new(output);
            loop {
                // Bounded line: at most 4 MiB PNG plus envelope. A broken helper
                // never grows unbounded memory or leaves a permit live at EOF.
                let mut bytes = Vec::new();
                match std::io::Read::by_ref(&mut reader)
                    .take(6 * 1024 * 1024)
                    .read_until(b'\n', &mut bytes)
                {
                    Ok(0) | Err(_) => break,
                    _ => {}
                }
                if bytes.last() != Some(&b'\n') {
                    break;
                }
                let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
                    break;
                };
                match v["event"].as_str() {
                    Some("paused") => {
                        u.store(v["reason"] == "banner", Ordering::SeqCst);
                        halt(&p);
                    }
                    Some("resumed") => {
                        u.store(false, Ordering::SeqCst);
                        p.store(false, Ordering::SeqCst);
                    }
                    Some("revoked") => halt(&r),
                    _ => {
                        if tx.send(v).is_err() {
                            break;
                        }
                    }
                }
            }
            halt(&r);
        });
        let client = Self {
            input: Mutex::new(input),
            child: Mutex::new(child),
            replies: Mutex::new(rx),
            paused,
            unproven,
            revoked,
            sequence: AtomicU64::new(0),
        };
        let ready = client
            .replies
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| unavailable())?;
        if ready["ready"] != true {
            client.stop();
            return Err(error(&ready));
        }
        Ok(client)
    }
    fn send(&self, v: Value) -> Result<(), ApiError> {
        let mut pipe = self.input.lock().map_err(|_| unavailable())?;
        writeln!(pipe, "{v}").map_err(|_| unavailable())?;
        pipe.flush().map_err(|_| unavailable())
    }
    pub fn stop(&self) {
        let _ = self.send(json!({"command":"revoke"}));
        let start = Instant::now();
        while !self.revoked.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(2));
        }
        if !self.revoked.load(Ordering::SeqCst) {
            if let Ok(mut child) = self.child.lock() {
                let _ = child.kill();
            }
        }
        self.revoked.store(true, Ordering::SeqCst);
    }
    pub fn live(&self) -> bool {
        !self.revoked.load(Ordering::SeqCst)
    }
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst) || self.revoked.load(Ordering::SeqCst)
    }
    pub fn unproven(&self) -> bool {
        self.unproven.load(Ordering::SeqCst)
    }
    pub fn resume(&self) -> Result<(), ApiError> {
        // The native permit increments its generation on takeover. Only commands
        // sent after this explicit UI operation can use the new generation.
        self.send(json!({"command":"resume"}))?;
        let start = Instant::now();
        while self.paused.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(2));
        }
        if !self.paused() {
            Ok(())
        } else if self.unproven() && self.live() {
            Err(ApiError::new(403, UNPROVEN))
        } else {
            Err(ApiError::new(403, STOPPED))
        }
    }
    pub fn action(
        &self,
        name: &str,
        args: &Value,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value, ApiError> {
        if self.paused() {
            return Err(ApiError::new(403, STOPPED));
        }
        let id = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        self.send(json!({"command":"action","id":id,"name":name,"arguments":args}))?;
        let start = Instant::now();
        loop {
            if cancel.load(Ordering::SeqCst) {
                self.stop();
                return Err(ApiError::new(
                    409,
                    "Action arrêtée. Un effet déjà livré ne peut pas être annulé.",
                ));
            }
            if self.revoked.load(Ordering::SeqCst) || start.elapsed() > Duration::from_secs(20) {
                self.stop();
                return Err(ApiError::new(409, STOPPED));
            }
            match self
                .replies
                .lock()
                .map_err(|_| unavailable())?
                .recv_timeout(Duration::from_millis(20))
            {
                Ok(v) if v["id"] == id => {
                    return if v.get("error").is_some() {
                        Err(error(&v))
                    } else {
                        Ok(v)
                    }
                }
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(unavailable()),
            }
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
