// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Anonymous bearer tokens, version 2. Pure functions: no database, no clock of their own.
//!
//! ```text
//! token   = "baze_v2." hex(payload) "." hex(mac)
//! payload = account_uuid[16] || issued_at_u32_be || expires_at_u32_be || key_id_u8     (25 bytes)
//! mac     = HMAC-SHA256(key[key_id], "baze/anon-token/v2\0" || payload)
//! ```
//!
//! * The token expires; a leaked one does not work forever.
//! * `key_id` is derived from the secret that signed the token (first byte of a labelled SHA-256), so the
//!   secret can be rotated: configure the new one as current and keep the old one as `previous`; tokens
//!   signed with either keep working until they expire, then the previous secret can be dropped.
//! * The MAC covers a protocol label, so the same secret can never validate a token for another purpose.
//! * Only one spelling of a token is valid: exact prefix, lowercase hex, exact lengths. An account
//!   can therefore not be presented under many different strings (rate-limit or cache evasion).

use chrono::{DateTime, Duration, TimeZone, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::fmt;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const PREFIX: &str = "baze_v2.";
const DOMAIN: &[u8] = b"baze/anon-token/v2\0";
const PAYLOAD_LEN: usize = 16 + 4 + 4 + 1;
const MAC_LEN: usize = 32;
/// How far in the future an `issued_at` may be before the token is refused (clock skew allowance).
const CLOCK_SKEW: Duration = Duration::seconds(60);

/// The secrets tokens are signed with: the current one, and optionally the previous one during a rotation.
#[derive(Clone)]
pub struct TokenKeys {
    current: (u8, Vec<u8>),
    previous: Option<(u8, Vec<u8>)>,
}

/// One byte naming a secret, so a verifier knows which configured secret to try. It is not secret and
/// reveals nothing useful about the key; two secrets sharing an id are both tried.
pub fn key_id(secret: &[u8]) -> u8 {
    let mut hasher = Sha256::new();
    hasher.update(b"baze/anon-token/key-id\0");
    hasher.update(secret);
    hasher.finalize()[0]
}

impl TokenKeys {
    pub fn new(current: impl Into<Vec<u8>>, previous: Option<Vec<u8>>) -> Self {
        let current = current.into();
        Self {
            current: (key_id(&current), current),
            previous: previous.map(|key| (key_id(&key), key)),
        }
    }

    /// The configured secrets whose id matches (at most two).
    fn candidates(&self, id: u8) -> impl Iterator<Item = &[u8]> {
        std::iter::once(&self.current)
            .chain(self.previous.as_ref())
            .filter(move |(candidate, _)| *candidate == id)
            .map(|(_, key)| key.as_slice())
    }
}

/// What a valid token asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    pub account_id: Uuid,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub key_id: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenError {
    /// Not a canonical v2 token.
    Malformed,
    /// Signed with a key this server does not have.
    UnknownKey,
    BadSignature,
    /// `issued_at` lies in the future beyond the clock skew allowance.
    NotYetValid,
    /// Authentic but past its expiry. Only reported after the signature checked out.
    Expired,
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed token",
            Self::UnknownKey => "unknown signing key",
            Self::BadSignature => "bad signature",
            Self::NotYetValid => "token issued in the future",
            Self::Expired => "token expired",
        })
    }
}

impl std::error::Error for TokenError {}

fn mac_for(key: &[u8], payload: &[u8]) -> HmacSha256 {
    // HMAC accepts keys of any length.
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(DOMAIN);
    mac.update(payload);
    mac
}

fn seconds(at: DateTime<Utc>) -> u32 {
    u32::try_from(at.timestamp().max(0)).unwrap_or(u32::MAX)
}

fn from_seconds(value: u32) -> DateTime<Utc> {
    Utc.timestamp_opt(i64::from(value), 0)
        .single()
        .expect("a u32 number of seconds is always a valid instant")
}

/// Issues a token for `account_id` valid for `ttl` from `now`, signed with the current secret.
pub fn issue(
    keys: &TokenKeys,
    account_id: Uuid,
    now: DateTime<Utc>,
    ttl: Duration,
) -> (String, Claims) {
    let issued_at = seconds(now);
    let expires_at = seconds(now + ttl);

    let mut payload = Vec::with_capacity(PAYLOAD_LEN);
    payload.extend_from_slice(account_id.as_bytes());
    payload.extend_from_slice(&issued_at.to_be_bytes());
    payload.extend_from_slice(&expires_at.to_be_bytes());
    payload.push(keys.current.0);

    let tag = mac_for(&keys.current.1, &payload).finalize().into_bytes();
    let token = format!("{PREFIX}{}.{}", hex::encode(&payload), hex::encode(tag));
    let claims = Claims {
        account_id,
        issued_at: from_seconds(issued_at),
        expires_at: from_seconds(expires_at),
        key_id: keys.current.0,
    };
    (token, claims)
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Checks a token. The signature is verified (in constant time) before anything else about the
/// token is trusted or reported, so the error cannot be used to probe which part was wrong.
pub fn verify(keys: &TokenKeys, token: &str, now: DateTime<Utc>) -> Result<Claims, TokenError> {
    let rest = token.strip_prefix(PREFIX).ok_or(TokenError::Malformed)?;
    let (payload_hex, mac_hex) = rest.split_once('.').ok_or(TokenError::Malformed)?;
    if payload_hex.len() != PAYLOAD_LEN * 2
        || mac_hex.len() != MAC_LEN * 2
        || !is_lower_hex(payload_hex)
        || !is_lower_hex(mac_hex)
    {
        return Err(TokenError::Malformed);
    }
    let payload = hex::decode(payload_hex).map_err(|_| TokenError::Malformed)?;
    let tag = hex::decode(mac_hex).map_err(|_| TokenError::Malformed)?;

    let key_id = payload[PAYLOAD_LEN - 1];
    let mut candidates = keys.candidates(key_id).peekable();
    if candidates.peek().is_none() {
        return Err(TokenError::UnknownKey);
    }
    // `verify_slice` compares in constant time.
    if !candidates.any(|key| mac_for(key, &payload).verify_slice(&tag).is_ok()) {
        return Err(TokenError::BadSignature);
    }

    let account_id = Uuid::from_slice(&payload[..16]).map_err(|_| TokenError::Malformed)?;
    let issued_at = from_seconds(u32::from_be_bytes(
        payload[16..20].try_into().expect("4 bytes"),
    ));
    let expires_at = from_seconds(u32::from_be_bytes(
        payload[20..24].try_into().expect("4 bytes"),
    ));

    if issued_at > now + CLOCK_SKEW {
        return Err(TokenError::NotYetValid);
    }
    if expires_at <= now {
        return Err(TokenError::Expired);
    }
    Ok(Claims {
        account_id,
        issued_at,
        expires_at,
        key_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> TokenKeys {
        TokenKeys::new(b"current-secret-for-tests-0123456789".to_vec(), None)
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
    }

    fn issue_at(keys: &TokenKeys, secs: i64) -> (String, Claims) {
        issue(keys, Uuid::new_v4(), at(secs), Duration::days(180))
    }

    /// Re-signs a payload with the given key, to build tokens that are authentic but unusual.
    fn forge(key: &[u8], payload: &[u8]) -> String {
        let tag = mac_for(key, payload).finalize().into_bytes();
        format!("{PREFIX}{}.{}", hex::encode(payload), hex::encode(tag))
    }

    fn payload(account: Uuid, issued: u32, expires: u32, key_id: u8) -> Vec<u8> {
        let mut p = account.as_bytes().to_vec();
        p.extend_from_slice(&issued.to_be_bytes());
        p.extend_from_slice(&expires.to_be_bytes());
        p.push(key_id);
        p
    }

    #[test]
    fn a_fresh_token_verifies_and_returns_its_claims() {
        let keys = keys();
        let account = Uuid::new_v4();
        let (token, issued) = issue(&keys, account, at(0), Duration::days(180));

        let claims = verify(&keys, &token, at(10)).unwrap();

        assert_eq!(claims, issued);
        assert_eq!(claims.account_id, account);
        assert_eq!(claims.expires_at - claims.issued_at, Duration::days(180));
        assert_eq!(
            claims.key_id,
            key_id(b"current-secret-for-tests-0123456789")
        );
    }

    #[test]
    fn the_token_has_the_documented_shape() {
        let (token, _) = issue_at(&keys(), 0);
        let rest = token.strip_prefix("baze_v2.").unwrap();
        let (payload, mac) = rest.split_once('.').unwrap();
        assert_eq!(payload.len(), 50);
        assert_eq!(mac.len(), 64);
        assert!(token.is_ascii());
    }

    #[test]
    fn expired_tokens_are_refused() {
        let keys = keys();
        let (token, claims) = issue_at(&keys, 0);
        assert_eq!(
            verify(&keys, &token, claims.expires_at),
            Err(TokenError::Expired)
        );
        assert_eq!(
            verify(&keys, &token, claims.expires_at + Duration::days(1)),
            Err(TokenError::Expired)
        );
        // One second earlier it still works.
        assert!(verify(&keys, &token, claims.expires_at - Duration::seconds(1)).is_ok());
    }

    #[test]
    fn tokens_from_the_future_are_refused_beyond_the_clock_skew() {
        let keys = keys();
        let (token, _) = issue_at(&keys, 3_600);
        assert_eq!(verify(&keys, &token, at(0)), Err(TokenError::NotYetValid));
        // A client clock a few seconds ahead is tolerated.
        let (near, _) = issue_at(&keys, 30);
        assert!(verify(&keys, &near, at(0)).is_ok());
    }

    #[test]
    fn a_token_signed_with_another_secret_is_refused() {
        let (token, _) = issue_at(&keys(), 0);
        let other = TokenKeys::new(b"a-completely-different-secret-value".to_vec(), None);
        // Different secrets almost always have different ids; if they happen to collide, the MAC fails.
        let result = verify(&other, &token, at(0));
        assert!(
            matches!(
                result,
                Err(TokenError::UnknownKey | TokenError::BadSignature)
            ),
            "{result:?}"
        );
    }

    #[test]
    fn changing_any_field_invalidates_the_signature() {
        let keys = keys();
        let (token, _) = issue_at(&keys, 0);
        let rest = token.strip_prefix(PREFIX).unwrap();
        let (payload_hex, mac_hex) = rest.split_once('.').unwrap();
        let payload = hex::decode(payload_hex).unwrap();

        // Flip one bit in each byte of the payload in turn: account id, issued_at, expires_at, key id.
        for index in 0..PAYLOAD_LEN {
            let mut tampered = payload.clone();
            tampered[index] ^= 0x01;
            let forged = format!("{PREFIX}{}.{mac_hex}", hex::encode(&tampered));
            let result = verify(&keys, &forged, at(0));
            assert!(
                matches!(
                    result,
                    Err(TokenError::BadSignature | TokenError::UnknownKey)
                ),
                "byte {index}: {result:?}"
            );
        }
        // And flip a bit of the MAC.
        let mut mac = hex::decode(mac_hex).unwrap();
        mac[0] ^= 0x80;
        let forged = format!("{PREFIX}{payload_hex}.{}", hex::encode(mac));
        assert_eq!(verify(&keys, &forged, at(0)), Err(TokenError::BadSignature));
    }

    #[test]
    fn the_mac_is_bound_to_the_protocol_label() {
        // The same secret signing the same payload WITHOUT the label (what a token for another
        // purpose, or the old v1 scheme, would look like) must not verify.
        let keys = keys();
        let p = payload(
            Uuid::new_v4(),
            seconds(at(0)),
            seconds(at(86_400)),
            keys.current.0,
        );
        let mut bare = HmacSha256::new_from_slice(&keys.current.1).unwrap();
        bare.update(&p);
        let token = format!(
            "{PREFIX}{}.{}",
            hex::encode(&p),
            hex::encode(bare.finalize().into_bytes())
        );
        assert_eq!(verify(&keys, &token, at(0)), Err(TokenError::BadSignature));
    }

    #[test]
    fn only_the_canonical_spelling_is_accepted() {
        let keys = keys();
        let (token, _) = issue_at(&keys, 0);
        let rest = token.strip_prefix(PREFIX).unwrap();
        let (payload_hex, mac_hex) = rest.split_once('.').unwrap();

        let variants = [
            format!("{PREFIX}{}.{mac_hex}", payload_hex.to_uppercase()),
            format!("{PREFIX}{payload_hex}.{}", mac_hex.to_uppercase()),
            format!("{} ", token),
            format!(" {token}"),
            format!("{token}\n"),
            format!("{PREFIX}{payload_hex}.{mac_hex}.extra"),
            format!("baze_v2:{payload_hex}.{mac_hex}"),
            format!("BAZE_V2.{payload_hex}.{mac_hex}"),
            format!(
                "{PREFIX}{}.{mac_hex}",
                &payload_hex[..payload_hex.len() - 2]
            ),
            format!("{PREFIX}{payload_hex}00.{mac_hex}"),
            format!("{PREFIX}{payload_hex}.{}", &mac_hex[..mac_hex.len() - 2]),
            format!("{PREFIX}{payload_hex}"),
            PREFIX.to_string(),
            String::new(),
        ];
        for variant in variants {
            assert_eq!(
                verify(&keys, &variant, at(0)),
                Err(TokenError::Malformed),
                "{variant:?}"
            );
        }
    }

    #[test]
    fn legacy_v1_tokens_are_not_valid() {
        let account = Uuid::new_v4();
        let mut mac = HmacSha256::new_from_slice(&keys().current.1).unwrap();
        mac.update(account.as_bytes());
        let v1 = format!(
            "baze_anon_{account}.{}",
            hex::encode(mac.finalize().into_bytes())
        );
        assert_eq!(verify(&keys(), &v1, at(0)), Err(TokenError::Malformed));
    }

    #[test]
    fn an_unknown_key_id_is_refused_even_with_a_valid_signature() {
        let keys = keys();
        let unknown = keys.current.0.wrapping_add(1);
        let p = payload(Uuid::new_v4(), seconds(at(0)), seconds(at(86_400)), unknown);
        let token = forge(&keys.current.1, &p);
        assert_eq!(verify(&keys, &token, at(0)), Err(TokenError::UnknownKey));
    }

    #[test]
    fn rotating_the_secret_keeps_existing_tokens_alive_until_they_expire() {
        let old_secret = b"the-old-secret-before-rotation-123456".to_vec();
        let new_secret = b"the-new-secret-after-rotation-abcdef".to_vec();
        assert_ne!(
            key_id(&old_secret),
            key_id(&new_secret),
            "pick fixtures with distinct ids"
        );

        // A token issued before the rotation, when the old secret was the current one.
        let (old_token, _) = issue_at(&TokenKeys::new(old_secret.clone(), None), 0);

        // After the rotation the new secret signs, the old one still verifies.
        let rotated = TokenKeys::new(new_secret.clone(), Some(old_secret));
        let claims = verify(&rotated, &old_token, at(10)).unwrap();
        assert_eq!(claims.key_id, rotated.previous.as_ref().unwrap().0);
        let (new_token, new_claims) = issue_at(&rotated, 10);
        assert_eq!(new_claims.key_id, key_id(&new_secret));
        assert!(verify(&rotated, &new_token, at(20)).is_ok());

        // Once the previous secret is dropped, tokens signed with it stop working.
        let after = TokenKeys::new(new_secret, None);
        assert_eq!(
            verify(&after, &old_token, at(10)),
            Err(TokenError::UnknownKey)
        );
        assert!(verify(&after, &new_token, at(20)).is_ok());
    }

    #[test]
    fn two_secrets_sharing_an_id_are_both_tried() {
        // Find two secrets whose one-byte ids collide (about 1 in 256), then check both still verify.
        let first = b"collision-search-seed-secret-00000000".to_vec();
        let second = (0u32..)
            .map(|n| format!("collision-search-candidate-{n:08}").into_bytes())
            .find(|candidate| key_id(candidate) == key_id(&first))
            .unwrap();
        let keys = TokenKeys::new(first.clone(), Some(second.clone()));

        let from_previous = forge(
            &second,
            &payload(
                Uuid::new_v4(),
                seconds(at(0)),
                seconds(at(86_400)),
                key_id(&second),
            ),
        );
        let (from_current, _) = issue_at(&keys, 0);

        assert!(verify(&keys, &from_previous, at(10)).is_ok());
        assert!(verify(&keys, &from_current, at(10)).is_ok());
    }

    #[test]
    fn expiry_is_reported_only_after_the_signature_is_checked() {
        // An attacker cannot learn that a forged token "would have been expired".
        let keys = keys();
        let p = payload(
            Uuid::new_v4(),
            seconds(at(-86_400 * 400)),
            seconds(at(-86_400 * 200)),
            keys.current.0,
        );
        let forged_expired = format!("{PREFIX}{}.{}", hex::encode(&p), "00".repeat(32));
        assert_eq!(
            verify(&keys, &forged_expired, at(0)),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn issuing_clamps_instead_of_overflowing() {
        let keys = keys();
        let far = Utc.timestamp_opt(4_290_000_000, 0).unwrap();
        let (_, claims) = issue(&keys, Uuid::new_v4(), far, Duration::days(730));
        assert_eq!(claims.expires_at.timestamp(), i64::from(u32::MAX));
    }
}
