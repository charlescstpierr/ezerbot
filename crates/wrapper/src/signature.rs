//! Fail-closed migration admission. Inspect only our own embedded signing
//! requirement; never enumerate identities, open a signing keychain or run a tool.
//! A cdhash pins a build, not an identity, so it cannot authorize migration.

#[derive(PartialEq, Eq)]
enum Token {
    Word(String),
    Quoted(String),
    Symbol(char),
}
fn tokens(text: &str) -> Option<Vec<Token>> {
    if text.len() > 16_384 {
        return None;
    }
    let mut chars = text.chars().peekable();
    let mut result = Vec::new();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if ch == '"' {
            let mut value = String::new();
            loop {
                match chars.next()? {
                    '"' => break,
                    '\\' => value.push(chars.next()?),
                    ch => value.push(ch),
                }
            }
            result.push(Token::Quoted(value));
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            let mut word = String::from(ch);
            while chars
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
            {
                word.push(chars.next().unwrap());
            }
            result.push(Token::Word(word.to_ascii_lowercase()));
        } else {
            result.push(Token::Symbol(ch));
        }
    }
    Some(result)
}
fn word(token: &Token, expected: &str) -> bool {
    matches!(token, Token::Word(value) if value == expected)
}
fn hash(token: &Token) -> bool {
    matches!(token, Token::Quoted(value) if value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()))
}
/// Deliberately conservative: require a positive, exact certificate/root pin,
/// never a build hash, a negation, or an alternative which could bypass the pin.
/// Identifiers/subjects inside quoted strings are data, not requirement syntax.
fn certificate_bound(text: &str) -> bool {
    let Some(tokens) = tokens(text) else {
        return false;
    };
    if tokens.iter().any(|t| {
        ["cdhash", "or", "not", "always", "never"]
            .iter()
            .any(|w| word(t, w))
            || matches!(t, Token::Symbol('!' | '|'))
    }) {
        return false;
    }
    tokens.windows(5).any(|p| {
        word(&p[0], "certificate")
            && (word(&p[1], "leaf") || word(&p[1], "0"))
            && p[2] == Token::Symbol('=')
            && word(&p[3], "h")
            && hash(&p[4])
    }) || tokens.windows(4).any(|p| {
        word(&p[0], "anchor") && p[1] == Token::Symbol('=') && word(&p[2], "h") && hash(&p[3])
    })
}

pub(super) fn stable_identity() -> bool {
    // Simulation is debug-only. The vault independently restricts debug native
    // access to the explicit throwaway keychain; its Default variant is absent.
    #[cfg(debug_assertions)]
    if let Ok(requirement) = std::env::var("GROKBOT_TEST_SIGNATURE_REQUIREMENT") {
        return certificate_bound(&requirement);
    }
    #[cfg(target_os = "macos")]
    {
        native::requirement().is_some_and(|text| certificate_bound(&text))
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
mod native {
    use core_foundation::{
        base::{CFTypeRef, TCFType},
        string::{CFString, CFStringRef},
    };
    use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};
    use std::ptr;

    // The safe bindings do not yet expose these two Copy APIs. This narrow FFI
    // lives in the executable only; the store and wrapper library forbid unsafe.
    // Both accept generic code objects, return +1 CF objects, and no key material.
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecCodeCopyDesignatedRequirement(
            code: CFTypeRef,
            flags: u32,
            requirement: *mut CFTypeRef,
        ) -> i32;
        fn SecRequirementCopyString(
            requirement: CFTypeRef,
            flags: u32,
            text: *mut CFStringRef,
        ) -> i32;
    }
    pub(super) fn requirement() -> Option<String> {
        let code = SecCode::for_self(Flags::NONE).ok()?;
        // A stable-looking custom DR on an unsigned/ad-hoc binary must not pass.
        let has_certificate: SecRequirement = "certificate leaf exists".parse().ok()?;
        code.check_validity(Flags::NO_NETWORK_ACCESS, &has_certificate)
            .ok()?;
        let mut raw = ptr::null();
        // SAFETY: live SecCode CF object; correctly sized out-pointer. The return
        // status and null pointer are checked before adopting the +1 reference.
        if unsafe { SecCodeCopyDesignatedRequirement(code.as_CFTypeRef(), 0, &mut raw) } != 0
            || raw.is_null()
        {
            return None;
        }
        // SAFETY: successful Copy returned a retained SecRequirement of this type.
        let requirement = unsafe { SecRequirement::wrap_under_create_rule(raw as _) };
        code.check_validity(Flags::NO_NETWORK_ACCESS, &requirement)
            .ok()?;
        let mut text = ptr::null();
        // SAFETY: valid retained requirement and CFStringRef out-pointer.
        if unsafe { SecRequirementCopyString(requirement.as_CFTypeRef(), 0, &mut text) } != 0
            || text.is_null()
        {
            return None;
        }
        // SAFETY: successful Copy returned a retained CFString, released by RAII.
        Some(unsafe { CFString::wrap_under_create_rule(text) }.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const HASH: &str = "0123456789ABCDEF0123456789ABCDEF01234567";
    #[test]
    fn exact_certificate_pin_survives_rebuild_but_cdhash_never_admits_migration() {
        for identifier in ["ca.ezerbot.wrapper.v1", "ca.ezerbot.wrapper.v2"] {
            assert!(certificate_bound(&format!(
                "identifier \"{identifier}\" and certificate leaf = H\"{HASH}\""
            )));
        }
        assert!(certificate_bound(&format!(
            "identifier \"local\" and anchor = H\"{HASH}\""
        )));
        assert!(!certificate_bound(&format!(
            "identifier \"local\" and cdhash H\"{HASH}\""
        )));
        assert!(!certificate_bound(""));
    }
    #[test]
    fn misleading_quoted_text_boolean_bypasses_and_unpinned_certificates_fail_closed() {
        for text in [
            format!("identifier \"certificate leaf = H\\\"{HASH}\\\"\""),
            format!("certificate leaf = H\"{HASH}\" or identifier \"other\""),
            format!("not certificate leaf = H\"{HASH}\""),
            format!("certificate leaf = H\"{HASH}\" and cdhash H\"{HASH}\""),
            "certificate leaf exists".into(),
            "certificate leaf = H\"short\"".into(),
            "identifier \"unterminated".into(),
        ] {
            assert!(!certificate_bound(&text));
        }
        assert!(certificate_bound(&format!(
            "identifier \"cdhash or not\" and certificate leaf = H\"{HASH}\""
        )));
    }
}
