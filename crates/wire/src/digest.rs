//! Empreintes de contenu, échangées en hexadécimal minuscule.

use grokbot_domain::ContentDigest;

use crate::WireError;

pub(crate) fn encode_digest(digest: &ContentDigest) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in digest.0 {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

pub(crate) fn decode_digest(hex: &str) -> Result<ContentDigest, WireError> {
    if hex.len() != 64 {
        return Err(WireError::InvalidDigest);
    }
    let bytes = hex.as_bytes();
    let mut decoded = [0u8; 32];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        decoded[index] = (high << 4) | low;
    }
    Ok(ContentDigest(decoded))
}

fn hex_nibble(byte: u8) -> Result<u8, WireError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(WireError::InvalidDigest),
    }
}
