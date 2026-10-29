//! Unicode Normalization Form C for UMP text (M2-104-UM 1.8, REQ-TEXT-001), with the feature
//! `nfc` (DEC-UMP-005). The writers of the core API take the text as the caller gives it; use
//! these helpers to check it or to make its NFC form before the write.

use alloc::string::String;
use unicode_normalization::UnicodeNormalization;

/// True when `s` is in Normalization Form C.
pub fn is_nfc(s: &str) -> bool {
    unicode_normalization::is_nfc(s)
}

/// The NFC form of `s`.
pub fn to_nfc(s: &str) -> String {
    s.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfc_check_and_normalization() {
        // DEC-UMP-005 verification: "e" + U+0301 is not NFC; its NFC form is U+00E9.
        assert!(!is_nfc("Cafe\u{301}"));
        assert_eq!(to_nfc("Cafe\u{301}"), "Caf\u{e9}");
        assert!(is_nfc("Caf\u{e9}"));
        assert!(is_nfc("Synth"));
        assert_eq!(to_nfc("Synth"), "Synth");
    }
}
