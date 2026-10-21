//! Release-only recorder transport. Debug builds never compile this module.
use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
fn unavailable() -> ApiError {
    ApiError::new(503,"Le helper de démonstration est indisponible. Ouvrez l’application native Ezerbot construite.")
}
fn executable() -> Result<PathBuf, ApiError> {
    Ok(std::env::current_exe()
        .map_err(|_| unavailable())?
        .parent()
        .ok_or_else(unavailable)?
        .join("Ezerbot"))
}
pub struct Recorder {
    child: Child,
    input: ChildStdin,
    samples: Receiver<Value>,
}
impl Recorder {
    pub fn start() -> Result<Self, ApiError> {
        // The person's checked consent + Commencer is the only entry point.
        if !Command::new(executable()?)
            .arg("--screen-permissions")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| unavailable())?
            .success()
        {
            return Err(ApiError::new(403,"Autorisez Accessibilité, Enregistrement de l’écran et Surveillance de l’entrée dans Réglages Système, puis recommencez la démonstration."));
        }
        let mut child = Command::new(executable()?)
            .args(["--demonstration-helper", &CAP.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| unavailable())?;
        let input = child.stdin.take().ok_or_else(unavailable)?;
        let output = child.stdout.take().ok_or_else(unavailable)?;
        let (tx, rx) = mpsc::sync_channel(4);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = vec![];
                if std::io::Read::by_ref(&mut reader)
                    .take(6 * 1024 * 1024)
                    .read_until(b'\n', &mut bytes)
                    .unwrap_or(0)
                    == 0
                    || bytes.last() != Some(&b'\n')
                {
                    break;
                }
                let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
                    break;
                };
                let stopped = v["stopped"] == true;
                if tx.send(v).is_err() || stopped {
                    return;
                }
            }
            let _ = tx.send(json!({"stopped":true,"error":"Le helper de capture a été interrompu. La démonstration est arrêtée."}));
        });
        let mut r = Self {
            child,
            input,
            samples: rx,
        };
        let ready = r
            .samples
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| unavailable())?;
        if ready["ready"] != true {
            r.stop();
            return Err(ApiError::new(
                403,
                ready["error"]
                    .as_str()
                    .unwrap_or("L’enregistrement n’a pas pu démarrer."),
            ));
        }
        Ok(r)
    }
    pub fn drain(&self) -> Vec<Value> {
        self.samples.try_iter().collect()
    }
    pub fn stop(&mut self) {
        let _ = writeln!(self.input, "{{\"command\":\"stop\"}}");
        let _ = self.input.flush();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(1) {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}
