//! Everything that must not be guessed, and how it is recognised afterwards.
//!
//! Device tokens and link codes are 32 bytes from the operating system's
//! random source, in base64url. The server keeps only their SHA-256: a copy of
//! the database gives nobody a token. They are found again by looking their
//! hash up, so no secret is ever compared byte by byte with a stored one; the
//! one comparison there is, the PKCE challenge, is in constant time.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hangar_cloud_api::{CHALLENGE_CHARS, MAX_VERIFIER_CHARS, MIN_VERIFIER_CHARS, TOKEN_PREFIX};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::refusal::Refusal;

/// A SHA-256, as the database keeps it (a 32-byte BLOB).
pub type Hash = [u8; 32];

/// The bytes of a secret: 256 bits.
const SECRET_BYTES: usize = 32;

/// 32 random bytes in base64url, 43 characters.
fn random_secret() -> Result<String, Refusal> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|err| Refusal::internal(format_args!("random: {err}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// A new device token: `hgr_` and 32 random bytes in base64url.
pub fn new_device_token() -> Result<String, Refusal> {
    Ok(format!("{TOKEN_PREFIX}{}", random_secret()?))
}

/// A new link code.
pub fn new_link_code() -> Result<String, Refusal> {
    random_secret()
}

/// A new state or PKCE verifier for a provider: 43 characters of base64url,
/// which is a verifier as RFC 7636 wants one.
pub fn new_oauth_secret() -> Result<String, Refusal> {
    random_secret()
}

/// The S256 challenge of a verifier: `base64url(sha256(verifier))`.
pub fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Whether `text` could be a secret this server drew: 43 characters of
/// base64url. What is not is never looked up.
pub fn is_secret_shaped(text: &str) -> bool {
    text.len() == SECRET_BYTES.div_ceil(3) * 4 - 1
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn sha256(text: &str) -> Hash {
    Sha256::digest(text.as_bytes()).into()
}

/// The hash a presented device token is looked up by, or nothing when the
/// text cannot be one of this server's tokens (and the database is not asked).
pub fn device_token_hash(token: &str) -> Option<Hash> {
    let secret = token.strip_prefix(TOKEN_PREFIX)?;
    let shaped = secret.len() == SECRET_BYTES.div_ceil(3) * 4 - 1
        && secret
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    shaped.then(|| sha256(token))
}

/// Whether `challenge` is what an S256 challenge looks like: 32 bytes in
/// base64url without padding.
pub fn is_challenge(challenge: &str) -> bool {
    challenge.len() == CHALLENGE_CHARS
        && URL_SAFE_NO_PAD
            .decode(challenge)
            .is_ok_and(|bytes| bytes.len() == 32)
}

/// Whether `verifier` is the one `challenge` was made from (RFC 7636, S256):
/// `base64url(sha256(verifier)) == challenge`, compared in constant time. A
/// verifier that is not one (its length, its characters) matches nothing.
pub fn verifier_matches(verifier: &str, challenge: &str) -> bool {
    let shaped = (MIN_VERIFIER_CHARS..=MAX_VERIFIER_CHARS).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'));
    let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let equal: bool = expected.as_bytes().ct_eq(challenge.as_bytes()).into();
    shaped & equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_token_is_the_prefix_and_43_characters_of_base64url() {
        let token = new_device_token().unwrap();
        let secret = token.strip_prefix("hgr_").unwrap();
        assert_eq!(secret.len(), 43);
        assert_eq!(URL_SAFE_NO_PAD.decode(secret).unwrap().len(), 32);
        assert_ne!(token, new_device_token().unwrap());
        assert_eq!(device_token_hash(&token), Some(sha256(&token)));
    }

    #[test]
    fn what_cannot_be_a_token_has_no_hash_to_look_up() {
        let token = new_device_token().unwrap();
        for text in [
            "",
            "hgr_",
            "hgr_short",
            &token[4..],
            &format!("{token}A"),
            &format!("{}=", &token[..token.len() - 1]),
            &format!("xyz_{}", &token[4..]),
        ] {
            assert_eq!(device_token_hash(text), None, "{text}");
        }
    }

    /// The example of RFC 7636, appendix B.
    #[test]
    fn the_verifier_of_the_rfc_matches_its_challenge_and_no_other() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(is_challenge(challenge));
        assert!(verifier_matches(verifier, challenge));
        assert!(!verifier_matches(
            verifier,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cN"
        ));
        assert!(!verifier_matches(&format!("{verifier}x"), challenge));
        assert!(!verifier_matches("", challenge));
        assert!(!verifier_matches(verifier, ""));
    }

    #[test]
    fn a_verifier_out_of_the_rfc_s_bounds_matches_nothing() {
        for verifier in ["short".to_string(), "a".repeat(129), "é".repeat(43)] {
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
            assert!(!verifier_matches(&verifier, &challenge), "{verifier}");
        }
        let longest = "a".repeat(128);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(longest.as_bytes()));
        assert!(verifier_matches(&longest, &challenge));
    }

    #[test]
    fn a_challenge_is_43_characters_of_base64url() {
        assert!(!is_challenge(""));
        assert!(!is_challenge(&"A".repeat(42)));
        assert!(!is_challenge(&"A".repeat(44)));
        assert!(!is_challenge(&"+".repeat(43)));
    }
}
