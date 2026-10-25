//! Identity of one desktop launch. The shell owns generation and delivery.
use std::io::Read;

pub const LAUNCH_HEADER: &str = "X-Ezerbot-Launch";

pub struct LaunchSecret(String);

impl LaunchSecret {
    pub fn read(mut channel: impl Read) -> Result<Self, &'static str> {
        let mut bytes = Vec::with_capacity(33);
        channel
            .by_ref()
            .take(33)
            .read_to_end(&mut bytes)
            .map_err(|_| "Le canal de lancement sécurisé est indisponible.")?;
        if bytes.len() != 32 {
            return Err("Le canal de lancement sécurisé est invalide.");
        }
        let value = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        bytes.fill(0);
        Ok(Self(value))
    }

    pub fn read_pipe() -> Result<Self, &'static str> {
        use rustix::fs::{fstat, FileType};
        let input = std::io::stdin();
        let metadata =
            fstat(&input).map_err(|_| "Le canal de lancement sécurisé est indisponible.")?;
        let private = match FileType::from_raw_mode(metadata.st_mode) {
            FileType::Fifo => true,
            FileType::Socket => {
                use std::os::fd::AsFd;
                let descriptor = input
                    .as_fd()
                    .try_clone_to_owned()
                    .map_err(|_| "Le canal de lancement sécurisé est indisponible.")?;
                let socket = std::os::unix::net::UnixStream::from(descriptor);
                // macOS discards the peer address when the sender closes to signal EOF.
                socket
                    .local_addr()
                    .is_ok_and(|address| address.is_unnamed())
            }
            _ => false,
        };
        if !private {
            return Err("Le canal de lancement sécurisé doit être un pipe privé.");
        }
        Self::read(input.lock())
    }

    pub fn header_value(&self) -> &str {
        &self.0
    }

    pub(crate) fn matches(&self, candidate: &str) -> bool {
        use sha2::{Digest, Sha256};
        let expected = Sha256::digest(self.0.as_bytes());
        let actual = Sha256::digest(candidate.as_bytes());
        expected
            .iter()
            .zip(actual.iter())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}
