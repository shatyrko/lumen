//! cargo run --example probe_cam2 -- <ip> <token> <did> : read-only probe of a camera's settings.
use std::net::Ipv4Addr;
use std::time::Duration;
use lumen::miio::{parse_token, Client};
use serde_json::json;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut c = Client::new(a[1].parse::<Ipv4Addr>().unwrap(), parse_token(&a[2]).unwrap()).unwrap();
    c.timeout = Duration::from_millis(1500);
    let did = a[3].clone();
    let names = ["power", "light", "full_color", "flip", "improve_program", "wdr", "track", "sdcard_status", "watermark", "max_client", "night_mode", "mini_level", "motion_record", "alarm", "alarm_sensitivity", "ptz", "motor", "sleep", "rotate", "privacy", "led", "volume", "speaker", "mic", "resolution", "osd", "auto_track", "motion_detect", "human_detect", "baby_cry", "nas_status", "record_mode", "time_zone"];
    for n in names {
        print!("{n}={:?}  ", c.call("get_prop", json!([n])).map(|v| v[0].clone()));
    }
    println!();
    for siid in 1..=8 {
        for piid in 1..=10 {
            if let Ok(v) = c.call("get_properties", json!([{"did": did, "siid": siid, "piid": piid}])) {
                if let Some(r) = v.get(0) {
                    if r["code"] == 0 {
                        println!("miot {siid}.{piid} = {}", r["value"]);
                    }
                }
            }
        }
    }
}
