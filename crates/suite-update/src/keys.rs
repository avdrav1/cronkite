//! The keys a release's `latest.json.sig` must verify with (ADR 0012, UPD-1): the two
//! release keys in `packaging/release-keys`, compiled in, and with the `test-key` feature
//! the test key in `test-key/` too.

use base64::Engine;

/// An Ed25519 public key's 32 bytes.
pub type PublicKey = [u8; 32];

/// The release keys' PEM files (`scripts/release_key.py` writes them).
const RELEASE_PEMS: [&str; 2] = [
    include_str!("../../../packaging/release-keys/release-1.pub"),
    include_str!("../../../packaging/release-keys/release-2.pub"),
];

/// The test key's public half; its private half is `test-key/test.key`, public on purpose.
#[cfg(any(test, feature = "test-key"))]
pub const TEST_PEM: &str = include_str!("../test-key/test.pub");

/// The keys this build trusts.
pub fn trusted() -> Vec<PublicKey> {
    #[allow(unused_mut)]
    let mut keys: Vec<PublicKey> = RELEASE_PEMS.iter().map(|p| key_of(p)).collect();
    #[cfg(feature = "test-key")]
    keys.push(key_of(TEST_PEM));
    keys
}

/// The 32 key bytes at the end of a PEM file's DER body: an Ed25519 SubjectPublicKeyInfo
/// (12 bytes of header, then the key) or a PKCS #8 private key (16, then the seed).
/// Panics on anything else: these files are compiled in, and the tests read every one.
pub fn key_of(pem: &str) -> [u8; 32] {
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .expect("a PEM file's body is base64");
    assert!(
        der.len() == 44 || der.len() == 48,
        "not an Ed25519 key ({} bytes)",
        der.len()
    );
    der[der.len() - 32..].try_into().expect("32 bytes")
}
