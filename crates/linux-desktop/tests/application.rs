#![cfg(debug_assertions)]
use grokbot_linux_desktop::{
    application::{deep_links, Inbox, Instance, Received, Remote},
    testing::{Home, PrivateBus},
    Bus, Unavailable, APPLICATION_ID,
};
use std::{
    collections::HashMap,
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    time::{Duration, Instant},
};
use zbus::{blocking::Proxy, zvariant::Value};

const WAIT: Duration = Duration::from_secs(5);

fn two_instances() -> Option<(Home, PrivateBus, Inbox, Remote)> {
    if !grokbot_linux_desktop::testing::tools(&["dbus-daemon"]) {
        return None;
    }
    let home = Home::new("eza");
    let bus = PrivateBus::new(&home);
    let Ok(Instance::Primary(inbox)) = Instance::claim(&bus.bus()) else {
        panic!("the first instance must own the name");
    };
    let Ok(Instance::Secondary(remote)) = Instance::claim(&bus.bus()) else {
        panic!("the second instance must reach the first");
    };
    Some((home, bus, inbox, remote))
}

#[test]
fn a_second_instance_hands_its_links_and_its_activation_to_the_first() {
    let Some((_home, bus, inbox, remote)) = two_instances() else {
        return;
    };
    let link = "ezerbot://app/v1/thread?id=abc".to_owned();
    remote.open(std::slice::from_ref(&link)).unwrap();
    assert_eq!(inbox.recv_timeout(WAIT), Some(Received::Open(link)));
    remote.activate().unwrap();
    assert_eq!(inbox.recv_timeout(WAIT), Some(Received::Activate));
    Proxy::new(
        &bus.connection(),
        APPLICATION_ID,
        "/ca/ezerbot/Ezerbot",
        "org.freedesktop.Application",
    )
    .unwrap()
    .call_method(
        "ActivateAction",
        &("open", Vec::<Value>::new(), HashMap::<&str, Value>::new()),
    )
    .unwrap();
    assert_eq!(inbox.recv_timeout(WAIT), Some(Received::Activate));
    assert_eq!(inbox.try_recv(), None);
}

#[test]
fn links_that_are_not_ezerbot_or_too_long_never_reach_the_inbox() {
    let Some((_home, _bus, inbox, remote)) = two_instances() else {
        return;
    };
    let prefix = "ezerbot://app/v1/open?text=";
    let long = format!("{prefix}{}", "a".repeat(2048));
    let at_limit = format!("{prefix}{}", "a".repeat(2048 - prefix.len()));
    assert_eq!(at_limit.len(), 2048);
    remote
        .open(&[
            "https://example.org/".into(),
            "file:///etc/passwd".into(),
            "ezerbot".into(),
            long,
            "EZERBOT://app/v1/settings?id=forms".into(),
            at_limit.clone(),
        ])
        .unwrap();
    assert_eq!(
        inbox.recv_timeout(WAIT),
        Some(Received::Open("EZERBOT://app/v1/settings?id=forms".into()))
    );
    assert_eq!(inbox.try_recv(), Some(Received::Open(at_limit)));
    assert_eq!(inbox.try_recv(), None);
}

#[test]
fn the_seventeenth_pending_link_is_refused_without_blocking() {
    let Some((_home, _bus, inbox, remote)) = two_instances() else {
        return;
    };
    let link = |n: usize| format!("ezerbot://app/v1/thread?id=t{n}");
    for n in 0..16 {
        remote.open(&[link(n)]).unwrap();
    }
    let started = Instant::now();
    assert_eq!(remote.open(&[link(16)]), Err(Unavailable));
    assert_eq!(remote.activate(), Err(Unavailable));
    assert!(started.elapsed() < Duration::from_secs(2));
    for n in 0..16 {
        assert_eq!(inbox.try_recv(), Some(Received::Open(link(n))));
    }
    assert_eq!(inbox.try_recv(), None);
    remote.open(&[link(17)]).unwrap();
    assert_eq!(inbox.recv_timeout(WAIT), Some(Received::Open(link(17))));
}

#[test]
fn a_refused_bus_never_claims_anything() {
    assert!(matches!(Instance::claim(&Bus::Refused), Err(Unavailable)));
}

#[test]
fn deep_links_keeps_at_most_sixteen_valid_utf8_links_in_order() {
    let mut args: Vec<OsString> = vec![
        "--flag".into(),
        "ezerbot://app/v1/settings?id=forms".into(),
        OsString::from_vec(b"ezerbot://app/\xff".to_vec()),
        "EzerBot://app/v1/agent?id=x".into(),
        "http://ezerbot.example/".into(),
        format!("ezerbot://{}", "a".repeat(2042)).into(),
    ];
    assert_eq!(
        deep_links(args.clone()),
        vec![
            "ezerbot://app/v1/settings?id=forms".to_owned(),
            "EzerBot://app/v1/agent?id=x".to_owned()
        ]
    );
    args.extend((0..30).map(|n| OsString::from(format!("ezerbot://app/v1/thread?id={n}"))));
    let links = deep_links(args);
    assert_eq!(links.len(), 16);
    assert_eq!(links[2], "ezerbot://app/v1/thread?id=0");
}
