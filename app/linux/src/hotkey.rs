use crate::portal::Portal;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use serde::Serialize;

pub const DISABLED: &str = "disabled";

/// One row per choice the wrapper stores, under the names it validates.
/// `cmd` is Super and `option` is Alt.
#[derive(Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub name: &'static str,
    /// The `preferred_trigger` the GlobalShortcuts portal is offered.
    pub trigger: &'static str,
    modifiers: Modifiers,
    code: Code,
}

const SUPER_ALT: Modifiers = Modifiers::SUPER.union(Modifiers::ALT);
const CONTROL_ALT: Modifiers = Modifiers::CONTROL.union(Modifiers::ALT);
const SUPER_SHIFT: Modifiers = Modifiers::SUPER.union(Modifiers::SHIFT);

pub const SHORTCUTS: [Shortcut; 4] = [
    Shortcut {
        name: "cmd+option+space",
        trigger: "LOGO+ALT+space",
        modifiers: SUPER_ALT,
        code: Code::Space,
    },
    Shortcut {
        name: "ctrl+option+space",
        trigger: "CTRL+ALT+space",
        modifiers: CONTROL_ALT,
        code: Code::Space,
    },
    Shortcut {
        name: "cmd+shift+space",
        trigger: "LOGO+SHIFT+space",
        modifiers: SUPER_SHIFT,
        code: Code::Space,
    },
    Shortcut {
        name: "cmd+option+j",
        trigger: "LOGO+ALT+j",
        modifiers: SUPER_ALT,
        code: Code::KeyJ,
    },
];

impl Shortcut {
    pub fn find(name: &str) -> Option<&'static Shortcut> {
        SHORTCUTS.iter().find(|shortcut| shortcut.name == name)
    }

    pub fn hotkey(&self) -> HotKey {
        HotKey::new(Some(self.modifiers), self.code)
    }
}

/// The same words macOS answers with, plus `unavailable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Registered,
    Disabled,
    Conflict,
    Error,
    Unavailable,
}

impl Outcome {
    /// `trigger` is what the desktop says it assigned, when it says.
    pub fn reply(self, trigger: Option<&str>) -> serde_json::Value {
        match trigger {
            Some(trigger) => serde_json::json!({ "status": self, "trigger": trigger }),
            None => serde_json::json!({ "status": self }),
        }
    }
}

/// Where a shortcut can be acquired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkeys {
    X11,
    Portal,
    Unavailable,
}

impl Hotkeys {
    /// XGrabKey under XWayland only sees keys while an X client has focus, so
    /// any Wayland session goes through the portal, whatever GDK runs on.
    pub fn choose(wayland_display: Option<&str>, session_type: Option<&str>, x11: bool) -> Self {
        let wayland = wayland_display.is_some_and(|value| !value.is_empty())
            || session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland"));
        if wayland {
            Self::Portal
        } else if x11 {
            Self::X11
        } else {
            Self::Unavailable
        }
    }

    pub fn detect() -> Self {
        use gtk::glib::prelude::ObjectExt;
        let x11 = gtk::gdk::Display::default()
            .is_some_and(|display| display.type_().name() == "GdkX11Display");
        Self::choose(
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            x11,
        )
    }
}

pub trait Grabs {
    fn acquire(&mut self, shortcut: &Shortcut) -> Result<(), Outcome>;
    fn release(&mut self, shortcut: &Shortcut);
}

/// The shortcut in force. A replacement is acquired before the old one is
/// released, and a failure touches nothing, so a conflict keeps the old one.
pub struct Slot {
    grabs: Box<dyn Grabs>,
    chosen: Option<&'static str>,
    held: Option<&'static Shortcut>,
}

impl Slot {
    pub fn new(grabs: Box<dyn Grabs>) -> Self {
        Self {
            grabs,
            chosen: None,
            held: None,
        }
    }

    pub fn configure(&mut self, name: &str) -> Outcome {
        if self.chosen == Some(name) {
            return self.state();
        }
        if name == DISABLED {
            if let Some(old) = self.held.take() {
                self.grabs.release(old);
            }
            self.chosen = Some(DISABLED);
            return Outcome::Disabled;
        }
        let Some(next) = Shortcut::find(name) else {
            return Outcome::Error;
        };
        if let Err(outcome) = self.grabs.acquire(next) {
            return outcome;
        }
        if let Some(old) = self.held.replace(next) {
            self.grabs.release(old);
        }
        self.chosen = Some(next.name);
        Outcome::Registered
    }

    fn state(&self) -> Outcome {
        if self.held.is_some() {
            Outcome::Registered
        } else {
            Outcome::Disabled
        }
    }

    pub fn fires(&self, id: u32) -> bool {
        self.held.is_some_and(|held| held.hotkey().id() == id)
    }
}

struct X11 {
    manager: GlobalHotKeyManager,
}

impl Grabs for X11 {
    fn acquire(&mut self, shortcut: &Shortcut) -> Result<(), Outcome> {
        self.manager
            .register(shortcut.hotkey())
            .map_err(|error| match error {
                global_hotkey::Error::AlreadyRegistered(_) => Outcome::Conflict,
                _ => Outcome::Error,
            })
    }

    fn release(&mut self, shortcut: &Shortcut) {
        let _ = self.manager.unregister(shortcut.hotkey());
    }
}

struct Nothing;

impl Grabs for Nothing {
    fn acquire(&mut self, _: &Shortcut) -> Result<(), Outcome> {
        Err(Outcome::Unavailable)
    }
    fn release(&mut self, _: &Shortcut) {}
}

impl Slot {
    /// `on_press` runs on the grab thread with the id of the key pressed.
    fn x11(on_press: impl Fn(u32) + Send + Sync + 'static) -> Self {
        // A dead grab thread would still answer `Ok`: only start it under X11.
        let Ok(manager) = GlobalHotKeyManager::new() else {
            return Self::new(Box::new(Nothing));
        };
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() == HotKeyState::Pressed {
                on_press(event.id());
            }
        }));
        Self::new(Box::new(X11 { manager }))
    }
}

/// Where the chosen backend keeps the shortcut in force.
pub enum Registrar {
    Slot(Slot),
    Portal(Portal),
}

impl Registrar {
    pub fn open(
        choice: Hotkeys,
        on_press: impl Fn(u32) + Send + Sync + 'static,
        on_portal: impl Fn(Option<String>) + 'static,
        on_lost: impl Fn() + 'static,
    ) -> Self {
        match choice {
            Hotkeys::X11 => Self::Slot(Slot::x11(on_press)),
            Hotkeys::Portal => Self::Portal(Portal::new(on_portal, on_lost)),
            Hotkeys::Unavailable => Self::Slot(Slot::new(Box::new(Nothing))),
        }
    }

    /// `done` runs at once for a grab, and after the portal answered for the portal.
    /// Only the portal can report a combination other than the one asked for.
    pub fn configure(&mut self, name: &str, done: impl FnOnce(Outcome, Option<String>) + 'static) {
        match self {
            Self::Slot(slot) => done(slot.configure(name), None),
            Self::Portal(portal) => portal.configure(name, done),
        }
    }

    pub fn fires(&self, id: u32) -> bool {
        matches!(self, Self::Slot(slot) if slot.fires(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Default)]
    struct Fake {
        log: Rc<RefCell<Vec<String>>>,
        taken: Vec<&'static str>,
    }

    impl Grabs for Fake {
        fn acquire(&mut self, shortcut: &Shortcut) -> Result<(), Outcome> {
            self.log
                .borrow_mut()
                .push(format!("acquire {}", shortcut.name));
            if self.taken.contains(&shortcut.name) {
                Err(Outcome::Conflict)
            } else {
                Ok(())
            }
        }
        fn release(&mut self, shortcut: &Shortcut) {
            self.log
                .borrow_mut()
                .push(format!("release {}", shortcut.name));
        }
    }

    fn slot(taken: &[&'static str]) -> (Slot, Rc<RefCell<Vec<String>>>) {
        let log = Rc::default();
        let fake = Fake {
            log: Rc::clone(&log),
            taken: taken.to_vec(),
        };
        (Slot::new(Box::new(fake)), log)
    }

    fn calls(log: &Rc<RefCell<Vec<String>>>) -> Vec<String> {
        log.borrow_mut().drain(..).collect()
    }

    #[test]
    fn the_replacement_is_acquired_before_the_old_one_is_released() {
        let (mut slot, log) = slot(&[]);
        assert_eq!(slot.configure("cmd+option+j"), Outcome::Registered);
        assert_eq!(slot.configure("ctrl+option+space"), Outcome::Registered);
        assert_eq!(
            calls(&log),
            [
                "acquire cmd+option+j",
                "acquire ctrl+option+space",
                "release cmd+option+j"
            ]
        );
    }

    #[test]
    fn a_failed_replacement_leaves_the_old_shortcut_in_force() {
        let (mut slot, log) = slot(&["cmd+shift+space"]);
        slot.configure("cmd+option+j");
        calls(&log);
        assert_eq!(slot.configure("cmd+shift+space"), Outcome::Conflict);
        assert_eq!(calls(&log), ["acquire cmd+shift+space"]);
        assert!(slot.fires(Shortcut::find("cmd+option+j").unwrap().hotkey().id()));
        assert!(!slot.fires(Shortcut::find("cmd+shift+space").unwrap().hotkey().id()));
        assert_eq!(slot.configure("cmd+option+j"), Outcome::Registered);
        assert!(
            calls(&log).is_empty(),
            "the held shortcut was grabbed again"
        );
    }

    #[test]
    fn disabling_releases_and_unknown_names_change_nothing() {
        let (mut slot, log) = slot(&[]);
        assert_eq!(slot.configure(DISABLED), Outcome::Disabled);
        slot.configure("cmd+option+j");
        calls(&log);
        for name in ["", "cmd+option+k", "DISABLED", "cmd+option+j "] {
            assert_eq!(slot.configure(name), Outcome::Error, "{name:?}");
        }
        assert!(calls(&log).is_empty());
        assert_eq!(slot.configure(DISABLED), Outcome::Disabled);
        assert_eq!(calls(&log), ["release cmd+option+j"]);
        assert_eq!(slot.configure(DISABLED), Outcome::Disabled);
        assert!(calls(&log).is_empty());
        assert!(!slot.fires(Shortcut::find("cmd+option+j").unwrap().hotkey().id()));
    }

    #[test]
    fn an_unavailable_backend_registers_nothing() {
        let mut slot = Slot::new(Box::new(Nothing));
        assert_eq!(slot.configure("cmd+option+j"), Outcome::Unavailable);
        assert_eq!(slot.configure(DISABLED), Outcome::Disabled);
        assert_eq!(slot.configure("cmd+option+j"), Outcome::Unavailable);
    }

    #[test]
    fn the_table_matches_the_names_the_wrapper_validates() {
        let wrapper = include_str!("../../../crates/wrapper/src/features/floating.rs");
        let start = wrapper.find("const SHORTCUTS").unwrap();
        let list = &wrapper[start..start + wrapper[start..].find("];").unwrap()];
        let mut stored: Vec<&str> = list.split('"').skip(1).step_by(2).collect();
        stored.sort_unstable();
        let mut ours: Vec<&str> = SHORTCUTS.iter().map(|shortcut| shortcut.name).collect();
        ours.push(DISABLED);
        ours.sort_unstable();
        assert_eq!(ours, stored);
        let ids: std::collections::HashSet<u32> = SHORTCUTS
            .iter()
            .map(|shortcut| shortcut.hotkey().id())
            .collect();
        assert_eq!(ids.len(), SHORTCUTS.len());
    }

    #[test]
    fn the_status_words_are_the_macos_ones() {
        let words: Vec<_> = [
            Outcome::Registered,
            Outcome::Disabled,
            Outcome::Conflict,
            Outcome::Error,
            Outcome::Unavailable,
        ]
        .map(|outcome| outcome.reply(None).to_string())
        .into();
        assert_eq!(
            words,
            [
                r#"{"status":"registered"}"#,
                r#"{"status":"disabled"}"#,
                r#"{"status":"conflict"}"#,
                r#"{"status":"error"}"#,
                r#"{"status":"unavailable"}"#,
            ]
        );
    }

    #[test]
    fn a_trigger_the_desktop_assigned_travels_with_the_status() {
        assert_eq!(
            Outcome::Registered.reply(Some("Ctrl Alt K")).to_string(),
            r#"{"status":"registered","trigger":"Ctrl Alt K"}"#
        );
    }

    #[test]
    fn wayland_sessions_use_the_portal_even_when_gdk_runs_on_x11() {
        use Hotkeys::*;
        let cases = [
            (None, None, true, X11),
            (None, None, false, Unavailable),
            (Some(""), Some("x11"), true, X11),
            (Some("wayland-0"), None, true, Portal),
            (Some("wayland-0"), Some("x11"), false, Portal),
            (None, Some("wayland"), true, Portal),
            (Some(""), Some("Wayland"), false, Portal),
            (None, Some("tty"), true, X11),
        ];
        for (display, session, x11, expected) in cases {
            assert_eq!(
                Hotkeys::choose(display, session, x11),
                expected,
                "{display:?} {session:?} {x11}"
            );
        }
    }
}
