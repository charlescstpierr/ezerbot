//! The protocol stream lives inside the lease state, so nothing can write a line
//! without seeing the phase, and a revoked lease has no stream left to write to.
use super::image::Encoded;
use serde_json::{json, Value};
use std::{
    io::Write,
    sync::{Mutex, MutexGuard},
    time::Instant,
};

const STOPPED: &str = "Partage en pause ou arrêté. Reprenez-le depuis l’application.";
const HIDDEN: &str =
    "Le bandeau de partage n’est pas visible : aucune capture tant qu’il ne l’est pas.";

pub struct Gate<W: Write>(Mutex<State<W>>);

enum State<W> {
    Open {
        out: W,
        phase: Phase,
        generation: u64,
        deadline: Instant,
        /// The overlay alone sets it, once the person can see the banner. No capture
        /// is admitted or delivered without it.
        banner: bool,
    },
    Closed,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Live,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stop {
    Pause,
    Revoke,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    Live,
    Paused,
    Closed,
}

/// Only `admit` mints one: every reply answers a request the lease accepted.
#[derive(Clone, Copy, Debug)]
pub struct Ticket {
    id: u64,
    generation: u64,
}

pub enum Reply {
    Image(Encoded),
    Refused(String),
}

impl<W: Write> State<W> {
    fn say(&mut self, line: Value) {
        let Self::Open { out, .. } = self else {
            return;
        };
        let mut bytes = line.to_string().into_bytes();
        bytes.push(b'\n');
        if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
            *self = Self::Closed;
        }
    }

    fn pause(&mut self) {
        if let Self::Open {
            phase, generation, ..
        } = self
        {
            if *phase == Phase::Live {
                *phase = Phase::Paused;
                *generation += 1;
                self.say(json!({"event":"paused"}));
            }
        }
    }

    fn revoke(&mut self) {
        self.say(json!({"event":"revoked"}));
        *self = Self::Closed;
    }

    /// The timer is not the only guard: a late tick cannot widen the lease.
    fn expire(&mut self) {
        if matches!(self, Self::Open { deadline, .. } if Instant::now() >= *deadline) {
            self.revoke();
        }
    }

    fn honors(&self, ticket: Ticket) -> bool {
        matches!(self, Self::Open { phase: Phase::Live, generation, banner: true, .. }
            if *generation == ticket.generation)
    }
}

impl<W: Write> Gate<W> {
    pub fn new(out: W, deadline: Instant) -> Self {
        Self(Mutex::new(State::Open {
            out,
            phase: Phase::Live,
            generation: 1,
            deadline,
            banner: false,
        }))
    }

    fn state(&self) -> MutexGuard<'_, State<W>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.expire();
        state
    }

    /// A banner not proven visible by now pauses the lease at once, so the app
    /// never announces a live share that refuses every capture.
    pub fn ready(&self) {
        let mut state = self.state();
        state.say(json!({"ready":true}));
        if matches!(*state, State::Open { banner: false, .. }) {
            state.pause();
        }
    }

    /// Ends the lease before it started: the wrapper reads this in place of `ready`.
    pub fn abort(&self, message: &str) {
        let mut state = self.state();
        state.say(json!({"error":message}));
        *state = State::Closed;
    }

    pub fn admit(&self, id: u64) -> Option<Ticket> {
        let mut state = self.state();
        match *state {
            State::Open {
                phase: Phase::Live,
                generation,
                banner: true,
                ..
            } => Some(Ticket { id, generation }),
            State::Open {
                phase: Phase::Live, ..
            } => {
                state.say(json!({"id":id,"error":HIDDEN}));
                None
            }
            _ => {
                state.say(json!({"id":id,"error":STOPPED}));
                None
            }
        }
    }

    pub fn current(&self, ticket: Ticket) -> bool {
        self.state().honors(ticket)
    }

    pub fn deliver(&self, ticket: Ticket, reply: Reply) {
        let mut state = self.state();
        let id = ticket.id;
        let line = match reply {
            Reply::Image(image) if state.honors(ticket) => json!({
                "id":id,"image":image.data_url,"width":image.width,"height":image.height
            }),
            Reply::Refused(reason) if state.honors(ticket) => json!({"id":id,"error":reason}),
            _ => json!({"id":id,"error":STOPPED}),
        };
        state.say(line);
    }

    pub fn stop(&self, stop: Stop) {
        let mut state = self.state();
        match (stop, &mut *state) {
            (Stop::Revoke, _) => state.revoke(),
            (Stop::Pause, _) => state.pause(),
        }
    }

    /// What the overlay can prove about its banner. Losing it while live pauses the
    /// lease, like the Pause button: covered, minimized or unprovable, it fails closed.
    /// Seeing it again resumes nothing; it only lets a resumed lease capture.
    pub fn banner(&self, visible: bool) {
        let mut state = self.state();
        let State::Open { banner, .. } = &mut *state else {
            return;
        };
        if std::mem::replace(banner, visible) && !visible {
            state.pause();
        }
    }

    pub fn showing(&self) -> bool {
        matches!(*self.state(), State::Open { banner: true, .. })
    }

    pub fn resume(&self) {
        let mut state = self.state();
        if let State::Open {
            phase, generation, ..
        } = &mut *state
        {
            if *phase == Phase::Paused {
                *phase = Phase::Live;
                *generation += 1;
            }
            state.say(json!({"event":"resumed"}));
        }
    }

    pub fn tick(&self) -> View {
        match *self.state() {
            State::Open {
                phase: Phase::Live, ..
            } => View::Live,
            State::Open { .. } => View::Paused,
            State::Closed => View::Closed,
        }
    }
}

#[cfg(test)]
pub mod sink {
    use serde_json::Value;
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    #[derive(Clone, Default)]
    pub struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Sink {
        pub fn lines(&self) -> Vec<Value> {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{sink::Sink, *};
    use std::{
        io,
        sync::{Arc, Barrier},
        time::Duration,
    };

    fn gate() -> (Gate<Sink>, Sink) {
        let sink = Sink::default();
        let deadline = Instant::now() + Duration::from_secs(60);
        let gate = Gate::new(sink.clone(), deadline);
        gate.banner(true);
        (gate, sink)
    }

    fn image() -> Reply {
        Reply::Image(Encoded {
            data_url: "data:image/png;base64,AA==".into(),
            width: 1280,
            height: 800,
        })
    }

    fn images(sink: &Sink) -> usize {
        sink.lines()
            .iter()
            .filter(|line| line.get("image").is_some())
            .count()
    }

    fn refused(sink: &Sink, id: u64) -> String {
        let lines = sink.lines();
        let line = lines.iter().rev().find(|line| line["id"] == id).unwrap();
        line["error"].as_str().unwrap().to_owned()
    }

    #[test]
    fn a_lease_ready_without_its_banner_says_it_is_paused() {
        let sink = Sink::default();
        let gate = Gate::new(sink.clone(), Instant::now() + Duration::from_secs(60));
        gate.ready();
        assert_eq!(
            sink.lines(),
            [json!({"ready":true}), json!({"event":"paused"})]
        );
        gate.banner(true);
        assert!(gate.admit(1).is_none(), "paused until the person resumes");
        gate.resume();
        assert!(gate.admit(2).is_some());
    }

    #[test]
    fn no_capture_is_admitted_before_the_banner_is_seen() {
        let sink = Sink::default();
        let gate = Gate::new(sink.clone(), Instant::now() + Duration::from_secs(60));
        assert!(gate.admit(1).is_none());
        assert_eq!(refused(&sink, 1), HIDDEN);
        gate.banner(true);
        assert!(gate.admit(2).is_some());
    }

    #[test]
    fn losing_the_banner_pauses_and_discards_the_capture_in_flight() {
        let (gate, sink) = gate();
        let ticket = gate.admit(1).unwrap();
        gate.banner(false);
        gate.deliver(ticket, image());
        assert_eq!(images(&sink), 0);
        assert_eq!(sink.lines()[0], json!({"event":"paused"}));
        gate.banner(true);
        assert!(
            gate.admit(2).is_none(),
            "seeing the banner again resumes nothing"
        );
        assert_eq!(refused(&sink, 2), STOPPED);
    }

    #[test]
    fn after_a_resume_nothing_is_captured_until_the_banner_is_back() {
        let (gate, sink) = gate();
        gate.banner(false);
        gate.resume();
        assert!(gate.admit(1).is_none());
        assert_eq!(refused(&sink, 1), HIDDEN);
        gate.banner(true);
        let ticket = gate.admit(2).unwrap();
        gate.deliver(ticket, image());
        assert_eq!(images(&sink), 1);
    }

    #[test]
    fn a_live_lease_delivers_the_reply_it_admitted() {
        let (gate, sink) = gate();
        gate.ready();
        let ticket = gate.admit(7).unwrap();
        gate.deliver(ticket, image());
        assert_eq!(
            sink.lines(),
            [
                json!({"ready":true}),
                json!({"id":7,"image":"data:image/png;base64,AA==","width":1280,"height":800})
            ]
        );
    }

    #[test]
    fn nothing_follows_a_revocation() {
        let (gate, sink) = gate();
        let ticket = gate.admit(1).unwrap();
        gate.stop(Stop::Revoke);
        gate.deliver(ticket, image());
        gate.deliver(ticket, Reply::Refused("trop tard".into()));
        assert!(gate.admit(2).is_none());
        gate.resume();
        gate.stop(Stop::Pause);
        gate.stop(Stop::Revoke);
        assert_eq!(sink.lines(), [json!({"event":"revoked"})]);
        assert_eq!(gate.tick(), View::Closed);
    }

    #[test]
    fn an_image_racing_a_revocation_either_precedes_it_or_never_exists() {
        for _ in 0..200 {
            let (gate, sink) = gate();
            let (gate, start) = (Arc::new(gate), Arc::new(Barrier::new(2)));
            let ticket = gate.admit(1).unwrap();
            let delivery = {
                let (gate, start) = (gate.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    gate.deliver(ticket, image());
                })
            };
            start.wait();
            gate.stop(Stop::Revoke);
            delivery.join().unwrap();
            assert_eq!(sink.lines().last(), Some(&json!({"event":"revoked"})));
        }
    }

    #[test]
    fn a_pause_during_a_capture_discards_it_even_after_resuming() {
        let (gate, sink) = gate();
        let before_pause = gate.admit(1).unwrap();
        gate.stop(Stop::Pause);
        gate.stop(Stop::Pause);
        assert!(!gate.current(before_pause));
        assert!(gate.admit(2).is_none());
        gate.deliver(before_pause, image());
        gate.resume();
        gate.deliver(before_pause, image());
        assert_eq!(images(&sink), 0);
        let after_resume = gate.admit(3).unwrap();
        assert!(gate.current(after_resume));
        gate.deliver(after_resume, image());
        assert_eq!(images(&sink), 1);
        let events: Vec<_> = sink
            .lines()
            .iter()
            .filter_map(|line| line["event"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(events, ["paused", "resumed"]);
    }

    #[test]
    fn a_refusal_of_a_live_request_keeps_the_lease() {
        let (gate, sink) = gate();
        let ticket = gate.admit(4).unwrap();
        gate.deliver(ticket, Reply::Refused("Capture annulée.".into()));
        assert_eq!(sink.lines(), [json!({"id":4,"error":"Capture annulée."})]);
        assert_eq!(gate.tick(), View::Live);
    }

    #[test]
    fn expiry_revokes_without_a_timer_and_while_paused() {
        let sink = Sink::default();
        let gate = Gate::new(sink.clone(), Instant::now() + Duration::from_millis(40));
        gate.banner(true);
        let ticket = gate.admit(1).unwrap();
        gate.stop(Stop::Pause);
        std::thread::sleep(Duration::from_millis(60));
        gate.deliver(ticket, image());
        assert_eq!(gate.tick(), View::Closed);
        assert_eq!(
            sink.lines(),
            [json!({"event":"paused"}), json!({"event":"revoked"})]
        );
    }

    #[test]
    fn a_failure_to_start_is_the_only_line_and_ends_the_lease() {
        let (gate, sink) = gate();
        gate.abort("Aucun affichage.");
        gate.ready();
        assert_eq!(sink.lines(), [json!({"error":"Aucun affichage."})]);
        assert_eq!(gate.tick(), View::Closed);
    }

    #[test]
    fn a_broken_stream_ends_the_lease() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let gate = Gate::new(Broken, Instant::now() + Duration::from_secs(60));
        gate.ready();
        assert_eq!(gate.tick(), View::Closed);
    }
}
