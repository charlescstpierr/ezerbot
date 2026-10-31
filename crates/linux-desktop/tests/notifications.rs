#![cfg(all(target_os = "linux", debug_assertions))]
use grokbot_linux_desktop::{
    notifications::{Alert, Notifier},
    testing::{FakeNotifications, Home, Notification, PrivateBus},
    Unavailable, APPLICATION_ID,
};
use std::{sync::mpsc::channel, time::Duration};

fn private_bus(label: &str) -> Option<(Home, PrivateBus)> {
    if !grokbot_linux_desktop::testing::tools(&["dbus-daemon"]) {
        return None;
    }
    let home = Home::new(label);
    let bus = PrivateBus::new(&home);
    Some((home, bus))
}

#[test]
fn a_notification_carries_the_title_the_fixed_body_a_default_action_and_the_desktop_entry() {
    let Some((_home, bus)) = private_bus("ezn") else {
        return;
    };
    let server = FakeNotifications::new(&bus);
    let notifier = Notifier::connect(&bus.bus()).unwrap();
    assert!(notifier.is_ready());
    let id = notifier
        .send(&Alert {
            title: "Réponse terminée",
            body: "Ouvrez Ezerbot pour consulter cet événement.",
        })
        .unwrap();
    assert_eq!(id, 1);
    assert_eq!(
        server.calls(),
        vec![Notification {
            app_name: "Ezerbot".into(),
            summary: "Réponse terminée".into(),
            body: "Ouvrez Ezerbot pour consulter cet événement.".into(),
            actions: vec!["default".into(), "Ouvrir".into()],
            desktop_entry: Some(APPLICATION_ID.into()),
            expire_timeout: -1,
        }]
    );
}

#[test]
fn only_the_default_action_reaches_the_click_handler() {
    let Some((_home, bus)) = private_bus("ezn") else {
        return;
    };
    let server = FakeNotifications::new(&bus);
    let notifier = Notifier::connect(&bus.bus()).unwrap();
    let (clicks, received) = channel();
    notifier
        .on_click(move |id| clicks.send(id).unwrap())
        .unwrap();
    let id = notifier
        .send(&Alert {
            title: "t",
            body: "b",
        })
        .unwrap();
    server.invoke(id, "ouvrir-autre");
    server.click(id);
    assert_eq!(received.recv_timeout(Duration::from_secs(5)), Ok(id));
    assert!(received.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn without_a_notification_server_nothing_is_ready_and_sending_fails() {
    let Some((_home, bus)) = private_bus("ezn") else {
        return;
    };
    let notifier = Notifier::connect(&bus.bus()).unwrap();
    assert!(!notifier.is_ready());
    assert_eq!(
        notifier.send(&Alert {
            title: "t",
            body: "b"
        }),
        Err(Unavailable)
    );
}
