//! A debug build keeps the fixture microphones: the recorders of the Linux audio
//! servers are not even present in the binary, so no device can be reached.
#![cfg(all(target_os = "linux", debug_assertions))]

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn debug_binary_cannot_record_from_hardware() {
    let binary = std::fs::read(env!("CARGO_BIN_EXE_grokbot-wrapper")).unwrap();
    assert!(
        contains(&binary, "wrapper_voice_microphone"),
        "the scan must read the real binary"
    );
    for recorder in ["pw-record", "parec", "pw-dump", "pactl"] {
        assert!(
            !contains(&binary, recorder),
            "{recorder} found in a debug build"
        );
    }
}
