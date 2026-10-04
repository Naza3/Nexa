//! Versioned server authentication bound to both endpoints of one TCP connection.
//! This does not protect against a same-user/admin attacker who can read the key.
use crate::token::SecretToken;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::net::{IpAddr, SocketAddr};

const DOMAIN: &[u8] = b"Nexa/local-http/server-proof\0";
pub const PROOF_PROTOCOL_VERSION: u32 = 1;
#[derive(Clone, Copy, Debug)]
pub struct ProofContext {
    pub instance_id: [u8; 16],
    pub nonce: [u8; 32],
    pub client: SocketAddr,
    pub server: SocketAddr,
}
pub fn normalize_endpoint(endpoint: SocketAddr) -> SocketAddr {
    match endpoint.ip() {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map_or(endpoint, |v4| SocketAddr::new(v4.into(), endpoint.port())),
        _ => endpoint,
    }
}
fn endpoint(mac: &mut Hmac<Sha256>, endpoint: SocketAddr) {
    let endpoint = normalize_endpoint(endpoint);
    match endpoint.ip() {
        IpAddr::V4(ip) => {
            mac.update(&[4]);
            mac.update(&ip.octets());
        }
        IpAddr::V6(ip) => {
            mac.update(&[6]);
            mac.update(&ip.octets());
        }
    }
    mac.update(&endpoint.port().to_be_bytes());
}
fn mac(token: &SecretToken, context: &ProofContext) -> Hmac<Sha256> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(token.key_bytes()).expect("HMAC accepts this fixed key");
    mac.update(DOMAIN);
    mac.update(&PROOF_PROTOCOL_VERSION.to_be_bytes());
    mac.update(&context.instance_id);
    mac.update(&context.nonce);
    endpoint(&mut mac, context.client);
    endpoint(&mut mac, context.server);
    mac
}
pub fn create_server_proof(token: &SecretToken, context: &ProofContext) -> [u8; 32] {
    mac(token, context).finalize().into_bytes().into()
}
pub fn verify_server_proof(token: &SecretToken, context: &ProofContext, proof: &[u8; 32]) -> bool {
    mac(token, context).verify_slice(proof).is_ok()
}
pub fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    result
}
pub fn decode_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 {
        return None;
    }
    let digit = |value| match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    };
    let mut result = [0; N];
    for (slot, pair) in result.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
        *slot = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc_4231_sha256_case_one() {
        let mut h = Hmac::<Sha256>::new_from_slice(&[0x0b; 20]).unwrap();
        h.update(b"Hi There");
        assert_eq!(
            encode_hex(&h.finalize().into_bytes()),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }
    #[test]
    fn every_connection_context_component_is_authenticated() {
        let token = SecretToken::generate().unwrap();
        let context = ProofContext {
            instance_id: [3; 16],
            nonce: [4; 32],
            client: "127.0.0.1:30000".parse().unwrap(),
            server: "127.0.0.1:18080".parse().unwrap(),
        };
        let proof = create_server_proof(&token, &context);
        assert!(verify_server_proof(&token, &context, &proof));
        let mut variants = [context; 6];
        variants[0].instance_id[0] ^= 1;
        variants[1].nonce[0] ^= 1;
        variants[2].client.set_port(30001);
        variants[3].server.set_port(18081);
        variants[4].client.set_ip("127.0.0.2".parse().unwrap());
        variants[5].server.set_ip("127.0.0.2".parse().unwrap());
        for variant in variants {
            assert!(!verify_server_proof(&token, &variant, &proof));
        }
        let mapped = ProofContext {
            client: "[::ffff:127.0.0.1]:30000".parse().unwrap(),
            ..context
        };
        assert!(verify_server_proof(&token, &mapped, &proof));
    }
    #[test]
    fn strict_hex_is_bounded_and_canonical() {
        assert_eq!(decode_hex::<2>("00ff"), Some([0, 255]));
        for value in ["00FF", "00fg", "00ff00", "0"] {
            assert!(decode_hex::<2>(value).is_none());
        }
    }
}
