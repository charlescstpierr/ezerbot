#[cfg(target_os = "linux")]
mod backend;
#[cfg(target_os = "linux")]
mod local_api;

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("ezerbot-linux est disponible uniquement sous Linux.");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("Ezerbot. {error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::backend::Backend;
    use gtk::prelude::*;
    use signal_hook::{
        consts::{SIGINT, SIGTERM},
        iterator::{Handle, Signals},
    };
    use std::{
        ffi::OsString,
        io::Cursor,
        path::PathBuf,
        process::{Command, Stdio},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread::JoinHandle,
        time::{Duration, Instant},
    };
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
        platform::unix::WindowExtUnix,
        window::WindowBuilder,
    };
    use wry::{
        NewWindowResponse, PermissionResponse, WebView, WebViewBuilder, WebViewBuilderExtUnix,
    };

    enum ShellEvent {
        Quit,
        LinkFailed(url::Url),
    }

    fn open_in_browser(url: url::Url, proxy: EventLoopProxy<ShellEvent>) {
        // xdg-open can last as long as the browser it starts.
        std::thread::spawn(move || {
            let status = Command::new("xdg-open")
                .arg(url.as_str())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if !status.is_ok_and(|status| status.success()) {
                let _ = proxy.send_event(ShellEvent::LinkFailed(url));
            }
        });
    }

    struct Shell {
        webview: Option<WebView>,
        backend: Backend,
    }

    impl Drop for Shell {
        fn drop(&mut self) {
            self.webview.take();
            self.backend.shutdown();
        }
    }

    struct SignalGuard {
        handle: Handle,
        thread: Option<JoinHandle<()>>,
    }
    impl SignalGuard {
        fn register(
            proxy: EventLoopProxy<ShellEvent>,
            quit: Arc<AtomicBool>,
        ) -> Result<Self, &'static str> {
            let mut signals = Signals::new([SIGTERM, SIGINT])
                .map_err(|_| "Les signaux de fermeture sont indisponibles.")?;
            let handle = signals.handle();
            let thread = std::thread::spawn(move || {
                for _ in signals.forever() {
                    quit.store(true, Ordering::SeqCst);
                    if proxy.send_event(ShellEvent::Quit).is_err() {
                        break;
                    }
                }
            });
            Ok(Self {
                handle,
                thread: Some(thread),
            })
        }
    }
    impl Drop for SignalGuard {
        fn drop(&mut self) {
            self.handle.close();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    struct Options {
        data_dir: Option<PathBuf>,
    }
    impl Options {
        fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, &'static str> {
            let mut arguments = arguments.into_iter();
            let mut data_dir = None;
            while let Some(argument) = arguments.next() {
                if argument != "--data-dir" || data_dir.is_some() {
                    return Err("Option inconnue. Utilisez --data-dir chemin pour un dossier de données isolé.");
                }
                let value = arguments
                    .next()
                    .filter(|value| !value.is_empty() && !value.to_string_lossy().starts_with("--"))
                    .ok_or("--data-dir attend un chemin.")?;
                data_dir = Some(PathBuf::from(value));
            }
            Ok(Self { data_dir })
        }

        fn data_directory(self) -> Result<PathBuf, &'static str> {
            if let Some(directory) = self.data_dir {
                return Ok(directory);
            }
            if let Some(directory) = std::env::var_os("XDG_DATA_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
            {
                return Ok(directory.join("ezerbot"));
            }
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/share/ezerbot"))
                .ok_or("Le dossier de données Ezerbot est indisponible.")
        }
    }

    pub fn run() -> Result<(), &'static str> {
        let directory = Options::parse(std::env::args_os().skip(1))?.data_directory()?;
        let wrapper = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("grokbot-wrapper")))
            .ok_or("Le service local Ezerbot est introuvable.")?;
        let event_loop = EventLoopBuilder::<ShellEvent>::with_user_event().build();
        let quit = Arc::new(AtomicBool::new(false));
        let signals = SignalGuard::register(event_loop.create_proxy(), Arc::clone(&quit))?;
        let backend = Backend::start(&wrapper, &directory, &quit)?;
        if quit.load(Ordering::SeqCst) {
            return Ok(());
        }
        let window = WindowBuilder::new()
            .with_title("Ezerbot")
            .with_inner_size(LogicalSize::new(1140, 780))
            .with_min_inner_size(LogicalSize::new(640, 480))
            .build(&event_loop)
            .map_err(|_| "La fenêtre Ezerbot n'a pas pu être créée.")?;
        let vbox = window
            .default_vbox()
            .ok_or("La fenêtre Ezerbot est indisponible.")?;
        let menubar = gtk::MenuBar::new();
        let application = gtk::MenuItem::with_label("Ezerbot");
        let menu = gtk::Menu::new();
        let close = gtk::MenuItem::with_label("Quitter");
        let accelerators = gtk::AccelGroup::new();
        window.gtk_window().add_accel_group(&accelerators);
        close.add_accelerator(
            "activate",
            &accelerators,
            *gtk::gdk::keys::constants::q,
            gtk::gdk::ModifierType::CONTROL_MASK,
            gtk::AccelFlags::VISIBLE,
        );
        let proxy = event_loop.create_proxy();
        close.connect_activate(move |_| {
            let _ = proxy.send_event(ShellEvent::Quit);
        });
        menu.append(&close);
        application.set_submenu(Some(&menu));
        menubar.append(&application);
        vbox.pack_start(&menubar, false, false, 0);
        menubar.show_all();
        if let Ok(icon) =
            gtk::gdk_pixbuf::Pixbuf::from_read(Cursor::new(include_bytes!("../../ui/icon-192.png")))
        {
            window.gtk_window().set_icon(Some(&icon));
        }
        let api = Arc::clone(&backend.api);
        let navigation = api.origin.clone();
        let downloads = api.origin.clone();
        let links = api.origin.clone();
        let link_proxy = event_loop.create_proxy();
        let resource_api = Arc::clone(&api);
        let webview = WebViewBuilder::new()
            .with_initialization_script_for_main_only(api.initialization_script(), true)
            .with_asynchronous_custom_protocol(
                "ezerbot-resource".into(),
                move |_, request, responder| Arc::clone(&resource_api).serve(request, responder),
            )
            .with_navigation_handler(move |url| navigation.allows_navigation(&url))
            .with_download_started_handler(move |url, _| downloads.allows_navigation(&url))
            .with_new_window_req_handler(move |url, _| {
                if let Some(url) = links.external_link(&url) {
                    open_in_browser(url, link_proxy.clone());
                }
                NewWindowResponse::Deny
            })
            .with_permission_handler(|_| PermissionResponse::Deny)
            .with_incognito(true)
            .with_devtools(false)
            .with_url(api.origin.as_str())
            .build_gtk(vbox)
            .map_err(|_| "La vue Ezerbot n'a pas pu être créée.")?;
        let mut shell = Some(Shell {
            webview: Some(webview),
            backend,
        });
        let failure_proxy = event_loop.create_proxy();
        let mut failure_dialog: Option<gtk::MessageDialog> = None;
        event_loop.run(move |event, _, control_flow| {
            let _keep_alive = (&window, &signals);
            *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250));
            let stopping = matches!(
                event,
                Event::UserEvent(ShellEvent::Quit)
                    | Event::WindowEvent {
                        event: WindowEvent::CloseRequested,
                        ..
                    }
                    | Event::LoopDestroyed
            );
            if let Event::UserEvent(ShellEvent::LinkFailed(url)) = &event {
                let dialog = gtk::MessageDialog::new(
                    Some(window.gtk_window()),
                    gtk::DialogFlags::DESTROY_WITH_PARENT,
                    gtk::MessageType::Error,
                    gtk::ButtonsType::Close,
                    "Le navigateur par défaut n'a pas pu ouvrir ce lien.",
                );
                dialog.set_secondary_text(Some(url.as_str()));
                dialog.connect_response(|dialog, _| dialog.close());
                dialog.show_all();
            } else if stopping {
                if let Some(dialog) = failure_dialog.take() {
                    dialog.close();
                }
                shell.take();
                *control_flow = ControlFlow::Exit;
            } else if matches!(event, Event::MainEventsCleared)
                && shell.as_mut().is_some_and(|shell| shell.backend.exited())
            {
                shell.take();
                let dialog = gtk::MessageDialog::new(
                    Some(window.gtk_window()),
                    gtk::DialogFlags::MODAL,
                    gtk::MessageType::Error,
                    gtk::ButtonsType::Close,
                    "Le service local Ezerbot s'est arrêté.",
                );
                let close_proxy = failure_proxy.clone();
                dialog.connect_response(move |dialog, _| {
                    dialog.close();
                    let _ = close_proxy.send_event(ShellEvent::Quit);
                });
                dialog.show_all();
                failure_dialog = Some(dialog);
            }
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn rejects_unknown_duplicate_and_incomplete_options() {
            for arguments in [
                vec!["--port", "123"],
                vec!["--data-dir"],
                vec!["--data-dir", "a", "--data-dir", "b"],
                vec!["--data-dir", "--unknown"],
                vec!["unexpected"],
            ] {
                assert!(Options::parse(arguments.into_iter().map(OsString::from)).is_err());
            }
            let options =
                Options::parse(["--data-dir", "/tmp/Ezerbot isolated"].map(OsString::from))
                    .unwrap();
            assert_eq!(
                options.data_dir.unwrap(),
                PathBuf::from("/tmp/Ezerbot isolated")
            );
        }
    }
}
