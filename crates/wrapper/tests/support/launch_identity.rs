use std::io::Write;
use std::process::{Child, Command, Stdio};

pub struct LaunchIdentity([u8; 32]);

impl LaunchIdentity {
    pub fn new() -> Self {
        let mut bytes = [0; 32];
        getrandom::getrandom(&mut bytes).expect("secure test identity");
        Self(bytes)
    }

    pub fn spawn(&self, command: &mut Command) -> Child {
        let mut child = command
            .stdin(Stdio::piped())
            .spawn()
            .expect("start test wrapper");
        let mut input = child.stdin.take().expect("private launch pipe");
        input.write_all(&self.0).expect("send launch identity");
        drop(input);
        child
    }

    pub fn value(&self) -> String {
        self.validator().header_value().to_owned()
    }

    pub fn validator(&self) -> grokbot_wrapper::LaunchSecret {
        grokbot_wrapper::LaunchSecret::read(self.0.as_slice()).expect("test launch identity")
    }
}
