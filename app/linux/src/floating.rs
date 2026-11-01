use crate::local_api::LocalApi;
use gtk::prelude::*;
use serde::Deserialize;
use std::cell::Cell;
use tao::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition},
    event_loop::EventLoopWindowTarget,
    platform::unix::{WindowBuilderExtUnix, WindowExtUnix},
    window::{Window, WindowBuilder},
};
use url::Url;
use wry::{
    http::Request, NewWindowResponse, PermissionResponse, WebView, WebViewBuilder,
    WebViewBuilderExtUnix,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Main,
    Panel,
}

/// The same identifier macOS accepts for `openConversation`.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct ConversationId(String);

impl TryFrom<String> for ConversationId {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let valid = (1..=128).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        valid
            .then_some(Self(value))
            .ok_or("Identifiant de conversation invalide.")
    }
}

impl ConversationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "command", rename_all = "camelCase", deny_unknown_fields)]
pub enum Command {
    // Empty struct variants: serde ignores unknown fields on unit variants.
    Show {},
    Configure {
        shortcut: String,
    },
    ConversationOpened {
        request: u64,
        opened: bool,
        #[serde(default)]
        reason: String,
    },
    Close {},
    Appearance {
        theme: Theme,
    },
    Drag {},
    OpenConversation {
        conversation_id: ConversationId,
    },
}

impl Command {
    fn view(&self) -> View {
        match self {
            Self::Show {} | Self::Configure { .. } | Self::ConversationOpened { .. } => View::Main,
            Self::Close {}
            | Self::Appearance { .. }
            | Self::Drag {}
            | Self::OpenConversation { .. } => View::Panel,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    launch: String,
    id: u64,
    body: serde_json::Value,
}

/// A command from a page that proved it may speak, or the refusal to answer it.
#[derive(Debug, PartialEq, Eq)]
pub struct Message {
    pub id: u64,
    pub command: Result<Command, &'static str>,
}

impl Message {
    /// `None` is a sender that never proved itself (wrong token or page): it
    /// gets no answer. A sender that did get an error it can show.
    pub fn parse(view: View, uri: &str, raw: &str, api: &LocalApi) -> Option<Self> {
        let envelope: Envelope = serde_json::from_str(raw).ok()?;
        let page = match view {
            View::Panel => api.origin.is_panel(uri),
            View::Main => {
                api.origin.allows_navigation(uri)
                    && Url::parse(uri).is_ok_and(|url| url.path() != "/floating")
            }
        };
        if !page || !api.authorizes(&envelope.launch) {
            return None;
        }
        let command = serde_json::from_value::<Command>(envelope.body)
            .map_err(|_| "Commande du panneau invalide.")
            .and_then(|command| {
                if command.view() == view {
                    Ok(command)
                } else {
                    Err("Commande refusée pour cette vue.")
                }
            });
        Some(Self {
            id: envelope.id,
            command,
        })
    }
}

/// What the page's pending promise is settled with. Values only ever reach
/// the page as JSON.
pub fn settle_script(id: u64, result: Result<serde_json::Value, &str>) -> String {
    let (value, error) = match result {
        Ok(value) => (value, None),
        Err(error) => (serde_json::Value::Null, Some(error)),
    };
    let error = serde_json::to_string(&error).unwrap_or_else(|_| "null".into());
    format!("window.EzerbotShell?.settle({id},{value},{error})")
}

pub fn open_conversation_script(request: u64, conversation: &ConversationId) -> String {
    let conversation = serde_json::to_string(conversation.as_str()).unwrap_or_default();
    format!("window.EzerbotShell?.openConversation({request},{conversation})")
}

pub fn settle(webview: &WebView, id: u64, result: Result<serde_json::Value, &str>) {
    let _ = webview.evaluate_script(&settle_script(id, result));
}

/// The compact window the global shortcut opens. It is hidden, never
/// destroyed, so a draft survives closing it.
pub struct Panel {
    // The view goes before its window.
    pub webview: WebView,
    pub window: Window,
    // Where it was last left. The window manager places a mapped window again
    // on its own terms, so the reopening has to say where.
    left_at: Cell<Option<PhysicalPosition<i32>>>,
}

impl Panel {
    pub fn open<T: 'static>(
        target: &EventLoopWindowTarget<T>,
        api: &LocalApi,
        ipc: impl Fn(Request<String>) + 'static,
    ) -> Result<Self, &'static str> {
        const ERROR: &str = "Le panneau flottant n'a pas pu être créé.";
        let window = WindowBuilder::new()
            .with_title("Panneau · Ezerbot")
            .with_inner_size(LogicalSize::new(420, 560))
            .with_position(LogicalPosition::new(80, 80))
            .with_decorations(false)
            .with_always_on_top(true)
            .with_resizable(false)
            .with_skip_taskbar(true)
            .with_visible_on_all_workspaces(true)
            .build(target)
            .map_err(|_| ERROR)?;
        let vbox = window.default_vbox().ok_or(ERROR)?;
        let origin = api.origin.clone();
        let webview = WebViewBuilder::new()
            .with_initialization_script_for_main_only(api.initialization_script(), true)
            .with_ipc_handler(ipc)
            .with_navigation_handler(move |url| origin.is_panel(&url))
            .with_download_started_handler(|_, _| false)
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_permission_handler(|_| PermissionResponse::Deny)
            .with_incognito(true)
            .with_devtools(false)
            .with_url(format!("{}/floating", api.origin.as_str()))
            .build_gtk(vbox)
            .map_err(|_| ERROR)?;
        let panel = Self {
            webview,
            window,
            left_at: Cell::new(None),
        };
        panel.show();
        Ok(panel)
    }

    pub fn show(&self) {
        self.window.set_visible(true);
        // Asked before the window manager has mapped it again, the focus goes
        // to the window that held it.
        let window = self.window.gtk_window().clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
            window.present();
        });
        let _ = self.webview.focus();
    }

    pub fn reopen(&self) {
        if let Some(at) = self.left_at.get() {
            self.window.set_outer_position(at);
        }
        let _ = self
            .webview
            .evaluate_script("window.EzerbotFloating?.open()");
        self.show();
        // The window manager drops this state when the window is withdrawn,
        // and GTK forgets it with it.
        self.window.set_always_on_top(true);
    }

    pub fn hide(&self) {
        // A window that is already hidden has no position worth keeping.
        if self.window.is_visible() {
            if let Ok(at) = self.window.outer_position() {
                self.left_at.set(Some(at));
            }
        }
        self.window.set_visible(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_api::BackendOrigin;
    use serde_json::json;

    const ORIGIN: &str = "http://127.0.0.1:32123";
    const TOKEN: &str = "synthetic-test-identity";

    fn api() -> LocalApi {
        LocalApi::new(BackendOrigin::parse(ORIGIN).unwrap(), TOKEN.into())
    }

    fn envelope(token: &str, body: serde_json::Value) -> String {
        json!({"launch": token, "id": 7, "body": body}).to_string()
    }

    fn parse(view: View, uri: &str, token: &str, body: serde_json::Value) -> Option<Message> {
        Message::parse(view, uri, &envelope(token, body), &api())
    }

    #[test]
    fn page_values_reach_the_script_only_as_json() {
        let hostile = "\"); alert(1); (\"</script>\u{2028}\n\\";
        let script = settle_script(4, Err(hostile));
        let argument = script
            .strip_prefix("window.EzerbotShell?.settle(4,null,")
            .and_then(|rest| rest.strip_suffix(')'))
            .unwrap();
        assert_eq!(serde_json::from_str::<String>(argument).unwrap(), hostile);
        assert!(!argument.contains('\n'));
        assert_eq!(
            settle_script(5, Ok(json!({"status": "conflict"}))),
            r#"window.EzerbotShell?.settle(5,{"status":"conflict"},null)"#
        );
        let id: ConversationId = "a_b-9".to_owned().try_into().unwrap();
        assert_eq!(
            open_conversation_script(2, &id),
            r#"window.EzerbotShell?.openConversation(2,"a_b-9")"#
        );
    }

    #[test]
    fn each_view_accepts_only_its_own_commands() {
        let panel = format!("{ORIGIN}/floating");
        let main = format!("{ORIGIN}/");
        let accepted = [
            (
                View::Main,
                &main,
                json!({"command": "show"}),
                Command::Show {},
            ),
            (
                View::Main,
                &main,
                json!({"command": "configure", "shortcut": "cmd+option+j"}),
                Command::Configure {
                    shortcut: "cmd+option+j".into(),
                },
            ),
            (
                View::Main,
                &main,
                json!({"command": "conversationOpened", "request": 3, "opened": false, "reason": "non"}),
                Command::ConversationOpened {
                    request: 3,
                    opened: false,
                    reason: "non".into(),
                },
            ),
            (
                View::Panel,
                &panel,
                json!({"command": "close"}),
                Command::Close {},
            ),
            (
                View::Panel,
                &panel,
                json!({"command": "appearance", "theme": "light"}),
                Command::Appearance {
                    theme: Theme::Light,
                },
            ),
            (
                View::Panel,
                &panel,
                json!({"command": "drag"}),
                Command::Drag {},
            ),
            (
                View::Panel,
                &panel,
                json!({"command": "openConversation", "conversation_id": "a-B_9"}),
                Command::OpenConversation {
                    conversation_id: "a-B_9".to_owned().try_into().unwrap(),
                },
            ),
        ];
        for (view, uri, body, command) in accepted {
            let message = parse(view, uri, TOKEN, body.clone()).unwrap();
            assert_eq!(message.id, 7);
            assert_eq!(message.command, Ok(command), "{body}");
            let other = if view == View::Main {
                (View::Panel, &panel)
            } else {
                (View::Main, &main)
            };
            let refused = parse(view, other.1, TOKEN, body.clone());
            assert!(refused.is_none(), "wrong page still spoke: {body}");
        }
    }

    #[test]
    fn a_command_of_the_other_view_is_refused_with_an_answer() {
        let panel = format!("{ORIGIN}/floating");
        let message = parse(View::Panel, &panel, TOKEN, json!({"command": "show"})).unwrap();
        assert_eq!(message.id, 7);
        assert!(message.command.is_err());
        let main = format!("{ORIGIN}/#reglages");
        let message = parse(View::Main, &main, TOKEN, json!({"command": "close"})).unwrap();
        assert!(message.command.is_err());
    }

    #[test]
    fn an_unproven_sender_gets_no_answer() {
        let panel = format!("{ORIGIN}/floating");
        let close = json!({"command": "close"});
        for token in ["", "synthetic-test-identitx", "synthetic-test-identity!"] {
            assert!(parse(View::Panel, &panel, token, close.clone()).is_none());
        }
        for uri in [
            "http://127.0.0.1:32123/floating?x=1",
            "http://127.0.0.1:32123/floating#x",
            "http://127.0.0.1:32123/floating/",
            "http://127.0.0.1:32124/floating",
            "https://outside.example/floating",
            "about:blank",
            "",
        ] {
            assert!(
                parse(View::Panel, uri, TOKEN, close.clone()).is_none(),
                "{uri}"
            );
        }
        for uri in [
            "http://127.0.0.1:32123/floating",
            "http://127.0.0.1:32123/floating?x",
            "http://127.0.0.1:32123/api/state",
            "http://127.0.0.1:32124/",
            "file:///etc/passwd",
        ] {
            let show = json!({"command": "show"});
            assert!(parse(View::Main, uri, TOKEN, show).is_none(), "{uri}");
        }
        for raw in [
            "",
            "{}",
            "[]",
            "null",
            "{\"launch\":1,\"id\":1,\"body\":{}}",
        ] {
            assert!(Message::parse(View::Panel, &panel, raw, &api()).is_none());
        }
        let extra = json!({"launch": TOKEN, "id": 1, "body": {"command": "close"}, "x": 1});
        assert!(Message::parse(View::Panel, &panel, &extra.to_string(), &api()).is_none());
        let negative = format!(r#"{{"launch":"{TOKEN}","id":-1,"body":{{"command":"close"}}}}"#);
        assert!(Message::parse(View::Panel, &panel, &negative, &api()).is_none());
    }

    #[test]
    fn invalid_bodies_are_answered_not_executed() {
        let panel = format!("{ORIGIN}/floating");
        let long = "a".repeat(129);
        for body in [
            json!({"command": "close", "extra": 1}),
            json!({"command": "show", "extra": 1}),
            json!({"command": "nonsense"}),
            json!({"command": "appearance", "theme": "sepia"}),
            json!({"command": "appearance"}),
            json!({"command": "openConversation"}),
            json!({"command": "openConversation", "conversation_id": ""}),
            json!({"command": "openConversation", "conversation_id": long}),
            json!({"command": "openConversation", "conversation_id": "a/b"}),
            json!({"command": "openConversation", "conversation_id": "a b"}),
            json!({"command": "openConversation", "conversation_id": "é"}),
            json!({"command": "openConversation", "conversation_id": 4}),
            json!({"command": ["close"]}),
            json!("close"),
            json!(null),
        ] {
            let message = parse(View::Panel, &panel, TOKEN, body.clone()).unwrap();
            assert!(message.command.is_err(), "{body}");
        }
        let main = format!("{ORIGIN}/");
        for body in [
            json!({"command": "configure"}),
            json!({"command": "configure", "shortcut": 4}),
            json!({"command": "conversationOpened", "opened": true}),
            json!({"command": "conversationOpened", "request": -1, "opened": true}),
        ] {
            let message = parse(View::Main, &main, TOKEN, body.clone()).unwrap();
            assert!(message.command.is_err(), "{body}");
        }
    }
}
