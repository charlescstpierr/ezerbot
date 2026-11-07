#[cfg(target_os = "linux")]
mod backend;
#[cfg(target_os = "linux")]
mod floating;
#[cfg(target_os = "linux")]
mod hotkey;
#[cfg(target_os = "linux")]
mod local_api;
#[cfg(target_os = "linux")]
mod screen;

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("ezerbot-linux est disponible uniquement sous Linux.");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    if let Some(invocation) = screen::Invocation::parse(std::env::args_os().skip(1)) {
        std::process::exit(screen::run(invocation));
    }
    if let Err(error) = linux::run() {
        eprintln!("Ezerbot. {error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::backend::{Backend, Instance};
    use crate::floating::{self, Command as PanelCommand, Message, Panel, View};
    use crate::hotkey::{Hotkeys, Outcome, Registrar};
    use crate::local_api::LocalApi;
    use gtk::prelude::*;
    use serde_json::json;
    use signal_hook::{
        consts::{SIGINT, SIGTERM},
        iterator::{Handle, Signals},
    };
    use std::{
        collections::HashMap,
        ffi::OsString,
        io::Cursor,
        path::PathBuf,
        process::{Command, Stdio},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        thread::JoinHandle,
        time::{Duration, Instant},
    };
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
        platform::unix::WindowExtUnix,
        window::{Window, WindowBuilder},
    };
    use wry::{
        http::Request, NewWindowResponse, PageLoadEvent, PermissionResponse, WebView,
        WebViewBuilder, WebViewBuilderExtUnix,
    };

    enum ShellEvent {
        Quit,
        Show,
        /// With the portal's activation token, when the portal sent one.
        OpenPanel(Option<String>),
        Hotkey(u32),
        Configured(u64, Outcome, Option<String>),
        /// The main page starts loading: a new page, with its bridge whole again.
        MainLoading,
        Command(View, Message),
        Link(String),
        PageLoaded,
        LinkFailed(url::Url),
    }

    /// Forwards what a page proved it may say, and nothing else.
    fn bridge(
        view: View,
        api: Arc<LocalApi>,
        proxy: EventLoopProxy<ShellEvent>,
    ) -> impl Fn(Request<String>) + 'static {
        move |request| {
            let uri = request.uri().to_string();
            if let Some(message) = Message::parse(view, &uri, request.body(), &api) {
                let _ = proxy.send_event(ShellEvent::Command(view, message));
            }
        }
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
        hotkeys: Registrar,
        panel: Option<Panel>,
        webview: Option<WebView>,
        instance: Option<Instance>,
        backend: Backend,
        proxy: EventLoopProxy<ShellEvent>,
        /// Panel requests waiting for the main window: its request, then the
        /// panel message to settle.
        opening: HashMap<u64, u64>,
        requests: u64,
    }

    impl Drop for Shell {
        fn drop(&mut self) {
            self.panel.take();
            self.webview.take();
            // A launch during the slow shutdown below starts afresh, and its
            // wrapper takes the engine once this one exits.
            self.instance.take();
            self.backend.shutdown();
        }
    }

    impl Shell {
        fn show_panel(
            &mut self,
            target: &EventLoopWindowTarget<ShellEvent>,
            token: Option<String>,
        ) -> Result<(), &'static str> {
            if let Some(panel) = &self.panel {
                panel.reopen(token);
                return Ok(());
            }
            let api = Arc::clone(&self.backend.api);
            let ipc = bridge(View::Panel, Arc::clone(&api), self.proxy.clone());
            self.panel = Some(Panel::open(target, &api, ipc, token)?);
            Ok(())
        }

        /// The page that was asked to open a thread is gone: its answer will never come.
        fn main_loading(&mut self) {
            for (_, panel_id) in std::mem::take(&mut self.opening) {
                self.settle(
                    View::Panel,
                    panel_id,
                    Err("La fenêtre principale s’est rechargée. Réessayez."),
                );
            }
        }

        fn settle(&self, view: View, id: u64, result: Result<serde_json::Value, &str>) {
            let webview = match view {
                View::Main => self.webview.as_ref(),
                View::Panel => self.panel.as_ref().map(|panel| &panel.webview),
            };
            if let Some(webview) = webview {
                floating::settle(webview, id, result);
            }
        }

        fn command(
            &mut self,
            target: &EventLoopWindowTarget<ShellEvent>,
            main: &Window,
            view: View,
            message: Message,
        ) {
            let reply = match message.command {
                Ok(command) => self.run(target, main, message.id, command),
                Err(error) => Some(Err(error)),
            };
            if let Some(reply) = reply {
                self.settle(view, message.id, reply);
            }
        }

        /// `None` is a command answered later.
        fn run(
            &mut self,
            target: &EventLoopWindowTarget<ShellEvent>,
            main: &Window,
            id: u64,
            command: PanelCommand,
        ) -> Option<Result<serde_json::Value, &'static str>> {
            Some(match command {
                PanelCommand::Show {} => self.show_panel(target, None).map(|()| json!(true)),
                PanelCommand::Configure { shortcut } => {
                    let proxy = self.proxy.clone();
                    self.hotkeys.configure(&shortcut, move |outcome, trigger| {
                        let _ = proxy.send_event(ShellEvent::Configured(id, outcome, trigger));
                    });
                    return None;
                }
                PanelCommand::ConversationOpened {
                    request,
                    opened,
                    reason,
                } => {
                    let Some(panel_id) = self.opening.remove(&request) else {
                        return Some(Err("Demande inconnue."));
                    };
                    if opened {
                        if let Some(panel) = &self.panel {
                            panel.hide();
                        }
                        main.gtk_window().present();
                        self.settle(View::Panel, panel_id, Ok(json!(true)));
                    } else {
                        let reason = if reason.is_empty() {
                            "Ce fil n’a pas pu être ouvert."
                        } else {
                            reason.as_str()
                        };
                        self.settle(View::Panel, panel_id, Err(reason));
                    }
                    Ok(json!(true))
                }
                PanelCommand::Close {} => {
                    if let Some(panel) = &self.panel {
                        panel.hide();
                    }
                    Ok(json!(true))
                }
                // The panel has no native chrome to theme.
                PanelCommand::Appearance { .. } => Ok(json!(true)),
                PanelCommand::Drag {} => {
                    if let Some(panel) = &self.panel {
                        let _ = panel.window.drag_window();
                    }
                    Ok(json!(true))
                }
                PanelCommand::OpenConversation { conversation_id } => {
                    let Some(webview) = &self.webview else {
                        return Some(Err("La fenêtre principale est indisponible."));
                    };
                    self.requests += 1;
                    self.opening.insert(self.requests, id);
                    let script =
                        floating::open_conversation_script(self.requests, &conversation_id);
                    let _ = webview.evaluate_script(&script);
                    return None;
                }
            })
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

    /// One request of `shell.fifo` holds one link, as long as the interface accepts.
    fn deep_link(argument: &OsString) -> Option<&str> {
        let link = argument.to_str()?;
        let scheme = link.get(.."ezerbot://".len())?;
        (scheme.eq_ignore_ascii_case("ezerbot://") && link.len() <= 2048 && !link.contains('\n'))
            .then_some(link)
    }

    /// The page takes the link once it has loaded. Until then, ask again.
    fn follow_link(webview: &WebView, link: String, retry: EventLoopProxy<ShellEvent>) {
        let script = format!(
            "Boolean(window.EzerbotDeepLinks?.receive({}))",
            serde_json::Value::from(link.as_str())
        );
        let _ = webview.evaluate_script_with_callback(&script, move |accepted| {
            if accepted != "true" {
                let (retry, link) = (retry.clone(), link.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(250));
                    let _ = retry.send_event(ShellEvent::Link(link));
                });
            }
        });
    }

    struct Options {
        data_dir: Option<PathBuf>,
        link: Option<String>,
    }
    impl Options {
        fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, &'static str> {
            let mut arguments = arguments.into_iter();
            let mut data_dir = None;
            let mut link = None;
            while let Some(argument) = arguments.next() {
                if let Some(raw) = deep_link(&argument).filter(|_| link.is_none()) {
                    link = Some(raw.to_owned());
                    continue;
                }
                if argument != "--data-dir" || data_dir.is_some() {
                    return Err("Option inconnue. Utilisez --data-dir chemin pour un dossier de données isolé.");
                }
                let value = arguments
                    .next()
                    .filter(|value| !value.is_empty() && !value.to_string_lossy().starts_with("--"))
                    .ok_or("--data-dir attend un chemin.")?;
                data_dir = Some(PathBuf::from(value));
            }
            Ok(Self { data_dir, link })
        }

        fn data_directory(&self) -> Result<PathBuf, &'static str> {
            if let Some(directory) = self.data_dir.clone() {
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
        let options = Options::parse(std::env::args_os().skip(1))?;
        let directory = options.data_directory()?;
        let wrapper = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("grokbot-wrapper")))
            .ok_or("Le service local Ezerbot est introuvable.")?;
        let event_loop = EventLoopBuilder::<ShellEvent>::with_user_event().build();
        let quit = Arc::new(AtomicBool::new(false));
        let signals = SignalGuard::register(event_loop.create_proxy(), Arc::clone(&quit))?;
        let Some(instance) = Instance::claim(&directory, options.link.as_deref())? else {
            return Ok(());
        };
        let show_proxy = event_loop.create_proxy();
        instance.watch(move |link| {
            let _ = show_proxy.send_event(ShellEvent::Show);
            if !link.is_empty() {
                let _ = show_proxy.send_event(ShellEvent::Link(link));
            }
        })?;
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
        let panel_item = gtk::MenuItem::with_label("Ouvrir le panneau flottant");
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
        let proxy = event_loop.create_proxy();
        panel_item.connect_activate(move |_| {
            let _ = proxy.send_event(ShellEvent::OpenPanel(None));
        });
        menu.append(&panel_item);
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
        let load_proxy = event_loop.create_proxy();
        let webview = WebViewBuilder::new()
            .with_initialization_script_for_main_only(api.initialization_script(), true)
            .with_on_page_load_handler(move |event, _| {
                let _ = load_proxy.send_event(match event {
                    PageLoadEvent::Started => ShellEvent::MainLoading,
                    PageLoadEvent::Finished => ShellEvent::PageLoaded,
                });
            })
            .with_ipc_handler(bridge(
                View::Main,
                Arc::clone(&api),
                event_loop.create_proxy(),
            ))
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
        floating::refuse_framed_documents(&webview);
        let hotkey_proxy = Mutex::new(event_loop.create_proxy());
        let hotkeys = Registrar::open(Hotkeys::detect(), move |id| {
            if let Ok(proxy) = hotkey_proxy.lock() {
                let _ = proxy.send_event(ShellEvent::Hotkey(id));
            }
        });
        let mut shell = Some(Shell {
            hotkeys,
            panel: None,
            webview: Some(webview),
            instance: Some(instance),
            backend,
            proxy: event_loop.create_proxy(),
            opening: HashMap::new(),
            requests: 0,
        });
        let link_retry = event_loop.create_proxy();
        // WebKit drops the answer to a script sent before the page has loaded.
        let mut waiting_links = Some(Vec::from_iter(options.link));
        let failure_proxy = event_loop.create_proxy();
        let mut failure_dialog: Option<gtk::MessageDialog> = None;
        event_loop.run(move |event, target, control_flow| {
            let _keep_alive = (&window, &signals);
            *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250));
            // Only the main window's close quits: the panel just hides.
            let stopping = match &event {
                Event::UserEvent(ShellEvent::Quit) | Event::LoopDestroyed => true,
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::CloseRequested,
                    ..
                } => *window_id == window.id(),
                _ => false,
            };
            let panel_closing = matches!(
                &event,
                Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. }
                    if shell
                        .as_ref()
                        .and_then(|shell| shell.panel.as_ref())
                        .is_some_and(|panel| panel.window.id() == *window_id)
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
            } else if matches!(event, Event::UserEvent(ShellEvent::Show)) {
                window.gtk_window().present();
            } else if let Event::UserEvent(ShellEvent::Command(view, message)) = event {
                if let Some(shell) = shell.as_mut() {
                    shell.command(target, &window, view, message);
                }
            } else if let Event::UserEvent(ShellEvent::Configured(id, outcome, trigger)) = &event {
                if let Some(shell) = &shell {
                    shell.settle(View::Main, *id, Ok(outcome.reply(trigger.as_deref())));
                }
            } else if let Event::UserEvent(ShellEvent::Hotkey(id)) = &event {
                // A key pressed after `shell.take()` finds no shell and does nothing.
                if let Some(shell) = shell.as_mut().filter(|shell| shell.hotkeys.fires(*id)) {
                    let _ = shell.show_panel(target, None);
                }
            } else if matches!(event, Event::UserEvent(ShellEvent::MainLoading)) {
                if let Some(shell) = shell.as_mut() {
                    shell.main_loading();
                }
            } else if let Event::UserEvent(ShellEvent::OpenPanel(token)) = &event {
                if let Some(shell) = shell.as_mut() {
                    let _ = shell.show_panel(target, token.clone());
                }
            } else if let Event::UserEvent(ShellEvent::Link(link)) = event {
                if let Some(waiting) = &mut waiting_links {
                    waiting.push(link);
                } else if let Some(webview) = shell.as_ref().and_then(|s| s.webview.as_ref()) {
                    follow_link(webview, link, link_retry.clone());
                }
            } else if matches!(event, Event::UserEvent(ShellEvent::PageLoaded)) {
                for link in waiting_links.take().into_iter().flatten() {
                    if let Some(webview) = shell.as_ref().and_then(|s| s.webview.as_ref()) {
                        follow_link(webview, link, link_retry.clone());
                    }
                }
            } else if panel_closing {
                if let Some(panel) = shell.as_ref().and_then(|shell| shell.panel.as_ref()) {
                    panel.hide();
                }
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
                vec!["ezerbot://app/a", "ezerbot://app/b"],
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
            for link in [
                "ezerbot://app/v1/thread?id=a",
                "EZERBOT://app/v1/thread?id=a",
            ] {
                let options = Options::parse([link].map(OsString::from));
                assert_eq!(options.unwrap().link.as_deref(), Some(link));
            }
            let long = format!("ezerbot://{}", "a".repeat(2048));
            assert!(Options::parse([OsString::from(long)]).is_err());
        }
    }
}
