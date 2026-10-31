#![cfg(all(target_os = "linux", debug_assertions))]
use grokbot_linux_desktop::{secrets::Collection, testing::Keyring, Unavailable};

const LABEL: &str = "Ezerbot — TOKEN";

#[test]
fn values_round_trip_through_the_encrypted_session_and_replace_in_place() {
    let Some(keyring) = Keyring::new() else {
        return;
    };
    let ours = Collection::open(&keyring.bus(), "scope-a".into()).unwrap();
    let other = Collection::open(&keyring.bus(), "scope-b".into()).unwrap();
    // Not UTF-8, and longer than one cipher block: a wrong key or padding would show.
    let binary: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    ours.put_verified("TOKEN", LABEL, b"fake-first").unwrap();
    ours.put_verified("TOKEN", LABEL, &binary).unwrap();
    ours.put_verified("OTHER", "Ezerbot — OTHER", b"fake-other")
        .unwrap();
    let mut entries = ours.entries().unwrap();
    entries.sort();
    assert_eq!(
        entries,
        vec![
            ("OTHER".to_owned(), b"fake-other".to_vec()),
            ("TOKEN".to_owned(), binary)
        ]
    );
    assert!(other.entries().unwrap().is_empty());
    let reconnected = Collection::open(&keyring.bus(), "scope-a".into()).unwrap();
    assert_eq!(reconnected.entries().unwrap().len(), 2);

    ours.delete_verified("TOKEN").unwrap();
    ours.delete_verified("TOKEN").unwrap();
    assert_eq!(
        reconnected.entries().unwrap(),
        vec![("OTHER".to_owned(), b"fake-other".to_vec())]
    );
}

#[test]
fn a_locked_collection_refuses_every_operation_and_writes_nothing() {
    let Some(mut keyring) = Keyring::new() else {
        return;
    };
    let collection = Collection::open(&keyring.bus(), "scope-a".into()).unwrap();
    collection
        .put_verified("TOKEN", LABEL, b"fake-before-lock")
        .unwrap();
    keyring.lock();

    assert!(matches!(
        Collection::open(&keyring.bus(), "scope-a".into()),
        Err(Unavailable)
    ));
    assert_eq!(collection.entries().unwrap_err(), Unavailable);
    assert_eq!(
        collection.put_verified("NEW", LABEL, b"fake-new"),
        Err(Unavailable)
    );
    assert_eq!(collection.delete_verified("TOKEN"), Err(Unavailable));

    keyring.unlock();
    let after = Collection::open(&keyring.bus(), "scope-a".into()).unwrap();
    assert_eq!(
        after.entries().unwrap(),
        vec![("TOKEN".to_owned(), b"fake-before-lock".to_vec())]
    );
}

#[test]
fn a_service_that_is_gone_is_unavailable() {
    let Some(mut keyring) = Keyring::new() else {
        return;
    };
    let collection = Collection::open(&keyring.bus(), "scope-a".into()).unwrap();
    keyring.stop();
    assert_eq!(collection.entries().unwrap_err(), Unavailable);
    assert!(Collection::open(&keyring.bus(), "scope-a".into()).is_err());
}
