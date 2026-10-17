//! cargo run --example probe_cam -- <ip> <token> : ask a camera a few read-only questions.
use std::net::Ipv4Addr;
use std::time::Duration;
use lumen::miio::{parse_token, Client};
use serde_json::json;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let ip: Ipv4Addr = a[1].parse().unwrap();
    let mut c = Client::new(ip, parse_token(&a[2]).unwrap()).unwrap();
    c.timeout = Duration::from_millis(1500);
    println!("hello: {:?}", c.handshake());
    for (m, p) in [
        ("miIO.info", json!([])),
        ("get_properties", json!([{"did": "x", "siid": 2, "piid": 1}])),
        ("get_prop", json!(["power"])),
    ] {
        let t = std::time::Instant::now();
        println!("{m}: {:?} ({} ms)", c.call(m, p), t.elapsed().as_millis());
    }
}
