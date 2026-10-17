//! Xiaomi's local protocol (miIO): JSON commands over UDP 54321, encrypted with the device's
//! 16-byte token (AES-128-CBC, key = md5(token), iv = md5(key + token)).
//!
//! Every packet has a 32-byte header: magic 0x2131, total length, 4 unknown bytes, the device
//! id, a stamp (the device's seconds counter) and an MD5 checksum over header + token + data.
//! A "hello" packet (all 0xff after the length) makes the device answer with its id and stamp.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use md5::{Digest, Md5};
use serde_json::{json, Value};

pub const PORT: u16 = 54321;

type Enc = cbc::Encryptor<aes::Aes128>;
type Dec = cbc::Decryptor<aes::Aes128>;

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// No answer (wrong address, light off at the wall, or asleep on Wi-Fi).
    Timeout,
    Io(String),
    /// The light answered with an error.
    Device(String),
    /// The answer could not be decrypted (wrong token: the light was reset or re-paired).
    BadToken,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Timeout => write!(f, "no answer"),
            Error::Io(e) => write!(f, "{e}"),
            Error::Device(e) => write!(f, "the light said: {e}"),
            Error::BadToken => write!(f, "wrong token (was the light reset?)"),
        }
    }
}

fn md5(parts: &[&[u8]]) -> [u8; 16] {
    let mut h = Md5::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

pub fn parse_token(hex: &str) -> Option<[u8; 16]> {
    let hex = hex.trim();
    if hex.len() != 32 {
        return None;
    }
    let mut t = [0u8; 16];
    for i in 0..16 {
        t[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(t)
}

#[derive(Clone)]
pub struct Crypto {
    token: [u8; 16],
    key: [u8; 16],
    iv: [u8; 16],
}

impl Crypto {
    pub fn new(token: [u8; 16]) -> Crypto {
        let key = md5(&[&token]);
        let iv = md5(&[&key, &token]);
        Crypto { token, key, iv }
    }

    pub fn encrypt(&self, plain: &[u8]) -> Vec<u8> {
        Enc::new(&self.key.into(), &self.iv.into()).encrypt_padded_vec_mut::<Pkcs7>(plain)
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, Error> {
        Dec::new(&self.key.into(), &self.iv.into()).decrypt_padded_vec_mut::<Pkcs7>(data).map_err(|_| Error::BadToken)
    }
}

pub fn hello_packet() -> [u8; 32] {
    let mut p = [0xffu8; 32];
    p[..4].copy_from_slice(&[0x21, 0x31, 0x00, 0x20]);
    p
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Header {
    pub device_id: u32,
    pub stamp: u32,
}

fn header_of(buf: &[u8]) -> Option<Header> {
    if buf.len() < 32 || buf[0] != 0x21 || buf[1] != 0x31 {
        return None;
    }
    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if len != buf.len() {
        return None;
    }
    Some(Header { device_id: u32::from_be_bytes(buf[8..12].try_into().ok()?), stamp: u32::from_be_bytes(buf[12..16].try_into().ok()?) })
}

/// The answer to a hello: just the header, no data.
pub fn parse_hello(buf: &[u8]) -> Option<Header> {
    if buf.len() != 32 {
        return None;
    }
    header_of(buf)
}

pub fn build(c: &Crypto, h: Header, payload: &[u8]) -> Vec<u8> {
    let enc = c.encrypt(payload);
    let mut p = Vec::with_capacity(32 + enc.len());
    p.extend_from_slice(&[0x21, 0x31]);
    p.extend_from_slice(&((32 + enc.len()) as u16).to_be_bytes());
    p.extend_from_slice(&0u32.to_be_bytes());
    p.extend_from_slice(&h.device_id.to_be_bytes());
    p.extend_from_slice(&h.stamp.to_be_bytes());
    let sum = md5(&[&p[..16], &c.token, &enc]);
    p.extend_from_slice(&sum);
    p.extend_from_slice(&enc);
    p
}

/// Check the checksum and decrypt. The JSON ends at the first zero byte.
pub fn parse(c: &Crypto, buf: &[u8]) -> Result<(Header, Vec<u8>), Error> {
    let h = header_of(buf).ok_or_else(|| Error::Io("not a miIO packet".into()))?;
    let data = &buf[32..];
    if md5(&[&buf[..16], &c.token, data]) != buf[16..32] {
        return Err(Error::BadToken);
    }
    let mut plain = c.decrypt(data)?;
    if let Some(z) = plain.iter().position(|b| *b == 0) {
        plain.truncate(z);
    }
    Ok((h, plain))
}

/// One light. Not thread-safe on purpose: each light has its own worker thread.
pub struct Client {
    sock: UdpSocket,
    pub addr: SocketAddr,
    crypto: Crypto,
    /// device id and stamp from the last packet, with when we saw it
    seen: Option<(Header, Instant)>,
    next_id: u32,
    pub timeout: Duration,
}

impl Client {
    pub fn new(ip: Ipv4Addr, token: [u8; 16]) -> Result<Client, Error> {
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| Error::Io(e.to_string()))?;
        // Ids must grow; the device answers a repeated id with its cached old answer.
        let next_id = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0) % 900_000) as u32 + 1000;
        Ok(Client { sock, addr: SocketAddr::from((ip, PORT)), crypto: Crypto::new(token), seen: None, next_id, timeout: Duration::from_millis(1200) })
    }

    pub fn set_ip(&mut self, ip: Ipv4Addr) {
        self.addr = SocketAddr::from((ip, PORT));
        self.seen = None;
    }

    fn recv(&self, buf: &mut [u8], until: Instant) -> Result<usize, Error> {
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Timeout);
            }
            self.sock.set_read_timeout(Some(left)).map_err(|e| Error::Io(e.to_string()))?;
            match self.sock.recv_from(buf) {
                Ok((n, from)) if from.ip() == self.addr.ip() => return Ok(n),
                Ok(_) => continue,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Err(Error::Timeout),
                Err(e) => return Err(Error::Io(e.to_string())),
            }
        }
    }

    pub fn handshake(&mut self) -> Result<Header, Error> {
        self.sock.send_to(&hello_packet(), self.addr).map_err(|e| Error::Io(e.to_string()))?;
        let mut buf = [0u8; 1024];
        let until = Instant::now() + self.timeout;
        loop {
            let n = self.recv(&mut buf, until)?;
            if let Some(h) = parse_hello(&buf[..n]) {
                self.seen = Some((h, Instant::now()));
                return Ok(h);
            }
        }
    }

    /// Send one command and wait for its answer's "result".
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let mut last = Error::Timeout;
        for _ in 0..3 {
            let (h, at) = match self.seen {
                Some(s) => s,
                // No hello answer: wrong address or the light is off at the wall. Retrying here
                // only delays the search for it at a new address.
                None => {
                    self.handshake()?;
                    self.seen.unwrap()
                }
            };
            let id = self.next_id;
            self.next_id += 1;
            let mut payload = serde_json::to_vec(&json!({ "id": id, "method": method, "params": params })).unwrap();
            payload.push(0);
            let stamp = h.stamp.wrapping_add(at.elapsed().as_secs() as u32).wrapping_add(1);
            let pkt = build(&self.crypto, Header { device_id: h.device_id, stamp }, &payload);
            if let Err(e) = self.sock.send_to(&pkt, self.addr) {
                return Err(Error::Io(e.to_string()));
            }
            let mut buf = [0u8; 4096];
            let until = Instant::now() + self.timeout;
            let answer = loop {
                let n = match self.recv(&mut buf, until) {
                    Ok(n) => n,
                    Err(e) => break Err(e),
                };
                if n == 32 {
                    continue; // a late hello answer
                }
                match parse(&self.crypto, &buf[..n]) {
                    Ok((rh, plain)) => {
                        let Ok(v) = serde_json::from_slice::<Value>(&plain) else { continue };
                        if v["id"].as_u64() != Some(id as u64) {
                            continue; // an answer to an older command
                        }
                        self.seen = Some((rh, Instant::now()));
                        break Ok(v);
                    }
                    Err(e) => break Err(e),
                }
            };
            match answer {
                Ok(v) => {
                    if let Some(err) = v.get("error") {
                        // -9999 "user ack timeout": the device wants a fresh, larger id
                        if err["code"].as_i64() == Some(-9999) {
                            self.next_id += 100;
                            last = Error::Device(err.to_string());
                            continue;
                        }
                        return Err(Error::Device(err["message"].as_str().map(String::from).unwrap_or_else(|| err.to_string())));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                Err(Error::BadToken) => return Err(Error::BadToken),
                Err(e) => {
                    self.seen = None;
                    last = e;
                }
            }
        }
        Err(last)
    }
}

/// Ask everything on the local network who speaks miIO: broadcast plus a sweep of our /24
/// (some routers drop broadcasts on Wi-Fi). Returns (device id, address) pairs; stops early
/// once every id in `want` has answered.
pub fn discover(near: Ipv4Addr, wait: Duration, want: &[u32]) -> Vec<(u32, Ipv4Addr)> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return Vec::new() };
    let _ = sock.set_broadcast(true);
    // Sends to addresses nobody has wait for ARP and would block; we do not care about those.
    let _ = sock.set_nonblocking(true);
    let hello = hello_packet();
    let _ = sock.send_to(&hello, (Ipv4Addr::BROADCAST, PORT));
    let local = local_ip_towards(near).unwrap_or(near);
    let o = local.octets();
    for last in 1..=254u8 {
        if last != o[3] {
            let _ = sock.send_to(&hello, (Ipv4Addr::new(o[0], o[1], o[2], last), PORT));
        }
    }
    let _ = sock.set_nonblocking(false);
    let mut found: Vec<(u32, Ipv4Addr)> = Vec::new();
    let until = Instant::now() + wait;
    let mut buf = [0u8; 256];
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let _ = sock.set_read_timeout(Some(left));
        match sock.recv_from(&mut buf) {
            Ok((n, SocketAddr::V4(from))) => {
                if let Some(h) = parse_hello(&buf[..n]) {
                    if !found.iter().any(|(d, _)| *d == h.device_id) {
                        found.push((h.device_id, *from.ip()));
                    }
                    if !want.is_empty() && want.iter().all(|w| found.iter().any(|(d, _)| d == w)) {
                        break;
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    found
}

/// Our own address on the network that leads to `ip` (no packet is sent).
pub fn local_ip_towards(ip: Ipv4Addr) -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect((ip, PORT)).ok()?;
    match s.local_addr().ok()? {
        SocketAddr::V4(a) => Some(*a.ip()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    // Built by python-miio 0.5.12 (Message.build) with the same token, id and stamp.
    const TOKEN: &str = "00112233445566778899aabbccddeeff";
    const PACKET: &str = "2131006000000000035b575c0012a2aa295f3b4344507589a3fce32ad37497c38201a5867fa00965f07b92be3ef26c5a9738b03abef0d4db1612576f13f16b6dc7851e50a0ace15411a08124ca52e65fcb19ba1b5cd345836c1d2ceac845553c";
    const JSON: &str = r#"{"id": 7, "method": "get_prop", "params": ["power", "bright"]}"#;

    #[test]
    fn builds_the_same_bytes_as_python_miio() {
        let c = Crypto::new(parse_token(TOKEN).unwrap());
        let mut payload = JSON.as_bytes().to_vec();
        payload.push(0);
        let p = build(&c, Header { device_id: 0x035b575c, stamp: 1221290 }, &payload);
        assert_eq!(p, unhex(PACKET));
    }

    #[test]
    fn parses_python_miio_packet() {
        let c = Crypto::new(parse_token(TOKEN).unwrap());
        let (h, plain) = parse(&c, &unhex(PACKET)).unwrap();
        assert_eq!(h, Header { device_id: 0x035b575c, stamp: 1221290 });
        assert_eq!(plain, JSON.as_bytes());
    }

    #[test]
    fn wrong_token_is_reported() {
        let c = Crypto::new([7u8; 16]);
        assert_eq!(parse(&c, &unhex(PACKET)).unwrap_err(), Error::BadToken);
    }

    #[test]
    fn hello_round_trip() {
        let h = hello_packet();
        assert_eq!(&h[..4], &[0x21, 0x31, 0x00, 0x20]);
        let mut reply = [0u8; 32];
        reply[..4].copy_from_slice(&[0x21, 0x31, 0x00, 0x20]);
        reply[8..12].copy_from_slice(&0x038c94d2u32.to_be_bytes());
        reply[12..16].copy_from_slice(&5u32.to_be_bytes());
        assert_eq!(parse_hello(&reply), Some(Header { device_id: 0x038c94d2, stamp: 5 }));
        assert_eq!(parse_hello(&reply[..31]), None);
    }

    #[test]
    fn tokens() {
        assert!(parse_token("zz112233445566778899aabbccddeeff").is_none());
        assert!(parse_token("0011").is_none());
        assert_eq!(parse_token(TOKEN).unwrap()[15], 0xff);
    }
}
