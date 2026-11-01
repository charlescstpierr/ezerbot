//! A private X server owned by the test that starts it, never the session's display.
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
};

pub struct Xvfb {
    server: Child,
    pub name: String,
}

impl Xvfb {
    pub fn start(geometry: &str) -> Self {
        let mut server = Command::new("Xvfb")
            .args(["-displayfd", "1", "-screen", "0", geometry])
            .args(["-nolisten", "tcp"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("Xvfb is required: sudo apt install xvfb");
        let mut number = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut number)
            .unwrap();
        let name = format!(":{}", number.trim());
        Self { server, name }
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}
