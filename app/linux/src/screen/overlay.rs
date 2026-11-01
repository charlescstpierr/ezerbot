//! Stays mapped for the whole lease. It receives clicks, unlike the macOS panel,
//! because no input tap exists here: its buttons are the takeover.
use super::gate::{Gate, Stop, View};
use gtk::{gdk, prelude::*};
use std::{cell::Cell, io::Stdout, sync::Arc};

const WIDTH: i32 = 560;
const HEIGHT: i32 = 44;
const LIVE: &str = "Ezerbot voit l’écran";
const BUTTON_WIDTH: i32 = 96;
const STYLE: &str = "window { background-color: #1f2129; }
label { color: #ffffff; font-weight: 600; }
button { background-image: none; background-color: #3a3d4a; border: 1px solid #5a5e6e;
  box-shadow: none; text-shadow: none; }
button:disabled { background-color: #2b2d36; }
button:disabled label { color: #8a8d99; }";

pub struct Overlay {
    window: gtk::Window,
    label: gtk::Label,
    pause: gtk::Button,
    shown: Cell<View>,
}

fn button(text: &str, gate: &Arc<Gate<Stdout>>, stop: Stop) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    button.set_size_request(BUTTON_WIDTH, -1);
    let gate = gate.clone();
    button.connect_clicked(move |_| gate.stop(stop));
    button
}

impl Overlay {
    pub fn build(gate: &Arc<Gate<Stdout>>) -> Self {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title(LIVE);
        window.set_decorated(false);
        window.set_keep_above(true);
        window.stick();
        window.set_skip_taskbar_hint(true);
        window.set_skip_pager_hint(true);
        window.set_type_hint(gdk::WindowTypeHint::Utility);
        window.set_focus_on_map(false);
        window.set_resizable(false);
        window.set_default_size(WIDTH, HEIGHT);
        let style = gtk::CssProvider::new();
        if style.load_from_data(STYLE.as_bytes()).is_ok() {
            if let Some(screen) = gdk::Screen::default() {
                gtk::StyleContext::add_provider_for_screen(
                    &screen,
                    &style,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
        }
        let label = gtk::Label::new(None);
        let pause = button("Pause", gate, Stop::Pause);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_border_width(6);
        row.pack_start(&label, true, true, 0);
        row.pack_start(&pause, false, false, 0);
        row.pack_start(&button("Arrêter", gate, Stop::Revoke), false, false, 0);
        window.add(&row);

        let closing = gate.clone();
        window.connect_delete_event(move |_, _| {
            closing.stop(Stop::Revoke);
            gtk::glib::Propagation::Stop
        });
        let minimized = gate.clone();
        window.connect_window_state_event(move |_, event| {
            if event
                .new_window_state()
                .contains(gdk::WindowState::ICONIFIED)
            {
                minimized.stop(Stop::Pause);
            }
            gtk::glib::Propagation::Proceed
        });
        let escape = gate.clone();
        window.connect_key_press_event(move |_, event| {
            if event.keyval() == gdk::keys::constants::Escape {
                escape.stop(Stop::Revoke);
            }
            gtk::glib::Propagation::Proceed
        });

        if let Some(area) = gdk::Display::default()
            .and_then(|display| display.monitor(0))
            .map(|monitor| monitor.geometry())
        {
            window.move_(area.x() + (area.width() - WIDTH) / 2, area.y() + 8);
        }
        label.set_text(LIVE);
        window.show_all();
        Self {
            window,
            label,
            pause,
            shown: Cell::new(View::Live),
        }
    }

    pub fn render(&self, view: View) {
        // Minimizing paused the lease: the overlay cannot stay hidden once the
        // bot sees the screen again, however quickly the app resumed.
        let minimized = |window: gdk::Window| window.state().contains(gdk::WindowState::ICONIFIED);
        if view == View::Live && self.window.window().is_some_and(minimized) {
            self.window.deiconify();
        }
        if self.shown.replace(view) == view {
            return;
        }
        self.pause.set_sensitive(view == View::Live);
        match view {
            View::Live => self.label.set_text(LIVE),
            View::Paused => self
                .label
                .set_text("Ezerbot en pause. Reprenez dans l’application."),
            View::Closed => self.window.hide(),
        }
    }
}
