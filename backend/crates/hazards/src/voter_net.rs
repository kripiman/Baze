// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which network a vote came from, kept as an opaque tag (ADR-0006 as amended, ADR-0008).
//!
//! Only the first vote from each /24 (IPv4) or /64 (IPv6) counts towards a report's threshold, so
//! the store has to remember the network of every ballot. It must not remember the address:
//! ADR-0005 forbids keeping network traces. The tag is therefore an HMAC of the network under a
//! server-side key, truncated to 128 bits. Equal networks give equal tags, which is all the
//! uniqueness rule needs; without the key the tag cannot be tested against a guessed network (a
//! plain hash of a /24 can be reversed by trying all 2^24 of them).

use hmac::{Hmac, Mac};
use sha2::Sha256;
use shared::{IpNet, network_of};
use std::fmt;
use std::net::IpAddr;

/// Bytes of the stored tag. Matches the `octet_length(voter_net) = 16` constraint.
pub const TAG_LEN: usize = 16;

/// Domain label: the same secret also signs tokens, and a MAC computed for one purpose must never be
/// valid for the other.
const DOMAIN: &[u8] = b"baze/voter-net/v1\0";

/// Representa la subred evaluada para la legitimidad del voto (/24 para IPv4, /64 para IPv6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VoterNetwork(IpNet);

impl VoterNetwork {
    const IPV4_PREFIX: u8 = 24;
    const IPV6_PREFIX: u8 = 64;

    pub fn from_ip(ip: IpAddr) -> Self {
        // `::ffff:a.b.c.d` is the same host as `a.b.c.d`; both must land in the same network.
        Self(network_of(
            ip.to_canonical(),
            Self::IPV4_PREFIX,
            Self::IPV6_PREFIX,
        ))
    }

    /// Unambiguous encoding: family, prefix length, network address.
    fn encode(&self) -> Vec<u8> {
        match self.0 {
            IpNet::V4(net) => [&[4, net.prefix_len()][..], &net.network().octets()[..]].concat(),
            IpNet::V6(net) => [&[6, net.prefix_len()][..], &net.network().octets()[..]].concat(),
        }
    }
}

/// The server-side key of the network tags.
///
/// Changing it changes every tag, so after a rotation a network can vote once more on the reports
/// that already exist (at most a TTL's worth of them). That is a bounded, accepted cost.
#[derive(Clone)]
pub struct VoterNetKey(Vec<u8>);

impl VoterNetKey {
    pub fn new(secret: &[u8]) -> Self {
        Self(secret.to_vec())
    }

    pub fn tag(&self, network: VoterNetwork) -> [u8; TAG_LEN] {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).expect("HMAC accepts any key length");
        mac.update(DOMAIN);
        mac.update(&network.encode());
        let full = mac.finalize().into_bytes();
        let mut tag = [0u8; TAG_LEN];
        tag.copy_from_slice(&full[..TAG_LEN]);
        tag
    }
}

impl fmt::Debug for VoterNetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VoterNetKey(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(ip: &str) -> VoterNetwork {
        VoterNetwork::from_ip(ip.parse().unwrap())
    }

    #[test]
    fn test_voter_network_prefixes() {
        assert_eq!(net("192.168.1.100").0.to_string(), "192.168.1.0/24");
        assert_eq!(
            net("2001:db8:85a3:0:1234:8a2e:370:7334").0.to_string(),
            "2001:db8:85a3::/64"
        );
    }

    #[test]
    fn an_ipv4_mapped_address_is_the_same_network_as_the_plain_one() {
        assert_eq!(net("::ffff:192.168.1.7"), net("192.168.1.200"));
    }

    #[test]
    fn hosts_of_one_network_share_a_tag_and_other_networks_do_not() {
        let key = VoterNetKey::new(b"a server secret");
        assert_eq!(key.tag(net("10.0.1.1")), key.tag(net("10.0.1.254")));
        assert_ne!(key.tag(net("10.0.1.1")), key.tag(net("10.0.2.1")));
        assert_eq!(
            key.tag(net("2001:db8:85a3:0:1::1")),
            key.tag(net("2001:db8:85a3:0:ffff::2"))
        );
        assert_ne!(
            key.tag(net("2001:db8:85a3:0::1")),
            key.tag(net("2001:db8:85a3:1::1"))
        );
    }

    #[test]
    fn the_tag_depends_on_the_key() {
        let one = VoterNetKey::new(b"first secret");
        let two = VoterNetKey::new(b"second secret");
        assert_ne!(one.tag(net("10.0.1.1")), two.tag(net("10.0.1.1")));
    }

    #[test]
    fn an_ipv4_and_an_ipv6_network_never_collide_by_encoding() {
        // 10.0.0.0/24 and the IPv6 network that starts with the same four bytes.
        let key = VoterNetKey::new(b"k");
        assert_ne!(key.tag(net("10.0.0.1")), key.tag(net("a00::1")));
    }

    #[test]
    fn the_tag_is_not_a_plain_hash_of_the_network() {
        use sha2::Digest;
        let key = VoterNetKey::new(b"k");
        let plain = Sha256::digest(net("10.0.1.1").encode());
        assert_ne!(&key.tag(net("10.0.1.1"))[..], &plain[..TAG_LEN]);
    }

    #[test]
    fn debug_output_does_not_reveal_the_key() {
        let text = format!("{:?}", VoterNetKey::new(b"hunter2-secret"));
        assert!(!text.contains("hunter2"), "{text}");
    }
}
