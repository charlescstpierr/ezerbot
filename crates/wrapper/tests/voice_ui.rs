//! Linux verifies persistence and the Speech bridge without Apple frameworks.
#[test]
fn dictation_locale_ui_contracts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("node")
        .current_dir(root)
        .args(["--test", "app/web/src/voice-ui.test.js"])
        .output()
        .expect("Node.js is required to verify the dictation bridge");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn speech_fixtures_are_valid_pcm() {
    for wav in [
        &include_bytes!("fixtures/dictation-fr.wav")[..],
        &include_bytes!("fixtures/dictation-en.wav")[..],
    ] {
        assert!(grokbot_wrapper::features::voice::wav_duration(wav).unwrap() > 1000);
    }
}

/// Mac only, opt-in: requires Speech permission and installed en-US/fr-CA locales.
///
/// Run it explicitly on a Mac with:
/// `cargo test -p grokbot-wrapper --test voice_ui -- --ignored french_audio_recognized_on_device_after_english`
#[test]
#[ignore = "opt-in Mac : permission Speech + locales en-US/fr-CA; lancer avec `cargo test -p grokbot-wrapper --test voice_ui -- --ignored`"]
fn french_audio_recognized_on_device_after_english() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(std::process::Command::new("bash")
        .current_dir(root)
        .arg("tools/test-dictation-fixture.sh")
        .status()
        .unwrap()
        .success());
}
