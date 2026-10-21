//! The iPhone client compares this denial verbatim. One Rust constant is the
//! source; this test fails if the server copies diverge from `app.js`.
#[test]
fn disabled_access_sentence_is_shared_with_the_client() {
    let message = grokbot_wrapper::features::iphone::ACCESS_DISABLED;
    let client = include_str!("../../../app/ui/app.js");
    assert!(
        client.contains(&format!("message === \"{message}\"")),
        "app.js must compare the server sentence exactly"
    );
    let quoted = format!("\"{message}\"");
    let copies = [
        include_str!("../src/main.rs"),
        include_str!("../src/features/iphone.rs"),
    ]
    .into_iter()
    .map(|source| source.matches(&quoted).count())
    .sum::<usize>();
    assert_eq!(
        copies, 1,
        "only ACCESS_DISABLED may quote the denial sentence"
    );
}
