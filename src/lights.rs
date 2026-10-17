//! What a light is (config), what it can do (by model), what state it is in, and the
//! commands, colour presets and scenes, in the Yeelight vocabulary that these lights speak
//! over miIO.

use std::net::Ipv4Addr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LightConfig {
    /// short name for the command line: desk, bulb, strip
    pub alias: String,
    /// the name in Mi Home
    pub name: String,
    pub model: String,
    pub ip: Ipv4Addr,
    /// Xiaomi device id (decimal); the light tells it in its hello answer, so it finds the
    /// light again when the router gives it another address
    pub did: String,
    pub token: String,
    #[serde(default)]
    pub mac: String,
}

impl LightConfig {
    pub fn device_id(&self) -> Option<u32> {
        self.did.parse().ok()
    }
    pub fn caps(&self) -> Caps {
        Caps::of(&self.model)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Lamp,
    Bulb,
    Strip,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caps {
    pub kind: Kind,
    pub color: bool,
    pub ct_min: u16,
    pub ct_max: u16,
    pub scenes: bool,
}

impl Caps {
    pub fn of(model: &str) -> Caps {
        match model {
            // Mi LED Desk Lamp 1S: refuses anything outside 2600-5000 K (tried on 2026-10-17)
            "yeelink.light.lamp4" => Caps { kind: Kind::Lamp, color: false, ct_min: 2600, ct_max: 5000, scenes: false },
            m if m.contains("strip") => Caps { kind: Kind::Strip, color: true, ct_min: 1700, ct_max: 6500, scenes: true },
            m if m.contains("lamp") || m.contains("mono") || m.contains("ceil") => Caps { kind: Kind::Lamp, color: false, ct_min: 2700, ct_max: 6500, scenes: false },
            m if m.contains("color") || m.contains("bslamp") => Caps { kind: Kind::Bulb, color: true, ct_min: 1700, ct_max: 6500, scenes: true },
            _ => Caps { kind: Kind::Bulb, color: false, ct_min: 2700, ct_max: 6500, scenes: false },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Color,
    White,
    Hsv,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LightState {
    /// None until the first answer; false after the light stopped answering
    pub online: Option<bool>,
    pub error: Option<String>,
    pub power: bool,
    pub bright: u8,
    pub ct: u16,
    pub rgb: u32,
    pub hue: u16,
    pub sat: u8,
    pub mode: Mode,
    pub flowing: bool,
    /// the scene we started, if one is running
    pub scene: Option<String>,
}

impl Default for LightState {
    fn default() -> Self {
        LightState { online: None, error: None, power: false, bright: 100, ct: 4000, rgb: 0xffffff, hue: 0, sat: 0, mode: Mode::White, flowing: false, scene: None }
    }
}

pub const PROPS: [&str; 8] = ["power", "bright", "ct", "rgb", "hue", "sat", "color_mode", "flowing"];

impl LightState {
    /// Fill from a get_prop answer (values come as strings, "" for what the model lacks).
    pub fn apply_props(&mut self, v: &Value) {
        let Some(a) = v.as_array() else { return };
        let get = |i: usize| a.get(i).and_then(|x| x.as_str().map(String::from).or_else(|| x.as_i64().map(|n| n.to_string()))).unwrap_or_default();
        self.power = get(0) == "on";
        if let Ok(b) = get(1).parse::<u8>() {
            self.bright = b.clamp(1, 100);
        }
        if let Ok(c) = get(2).parse() {
            self.ct = c;
        }
        if let Ok(c) = get(3).parse() {
            self.rgb = c;
        }
        if let Ok(h) = get(4).parse() {
            self.hue = h;
        }
        if let Ok(s) = get(5).parse() {
            self.sat = s;
        }
        self.mode = match get(6).as_str() {
            "1" => Mode::Color,
            "3" => Mode::Hsv,
            _ => Mode::White,
        };
        self.flowing = get(7) == "1";
        if !self.flowing {
            self.scene = None;
        }
    }

    /// The colour the light shows now, for the dot in the panel.
    pub fn shown_rgb(&self) -> (u8, u8, u8) {
        match self.mode {
            Mode::Color => split(self.rgb),
            Mode::Hsv => hsv_to_rgb(self.hue as f64, self.sat as f64 / 100.0, 1.0),
            Mode::White => kelvin_rgb(self.ct),
        }
    }
}

pub fn split(rgb: u32) -> (u8, u8, u8) {
    ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub fn join(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let h = (h % 360.0 + 360.0) % 360.0 / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8)
}

pub fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    ((h + 360.0) % 360.0, if max == 0.0 { 0.0 } else { d / max }, max)
}

/// Roughly the colour of white light at a colour temperature (Tanner Helland's fit).
pub fn kelvin_rgb(k: u16) -> (u8, u8, u8) {
    let t = k as f64 / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.698727446 * (t - 60.0).powf(-0.1332047592) };
    let g = if t <= 66.0 { 99.4708025861 * t.ln() - 161.1195681661 } else { 288.1221695283 * (t - 60.0).powf(-0.0755148492) };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.5177312231 * (t - 10.0).ln() - 305.0447927307
    };
    (r.clamp(0.0, 255.0) as u8, g.clamp(0.0, 255.0) as u8, b.clamp(0.0, 255.0) as u8)
}

/// What one can ask a light to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Power(bool),
    Toggle,
    Bright(u8),
    Ct(u16),
    Rgb(u32),
    Hsv(u16, u8),
    Scene(String),
    StopScene,
    /// brightness and temperature together (white presets)
    White(u8, u16),
}

impl Action {
    /// Actions of the same slot replace each other while waiting (a dragged slider sends many).
    pub fn slot(&self) -> u8 {
        match self {
            Action::Power(_) | Action::Toggle => 0,
            Action::Bright(_) => 1,
            Action::Ct(_) | Action::Rgb(_) | Action::Hsv(..) | Action::Scene(_) | Action::StopScene | Action::White(..) => 2,
        }
    }

    /// The miIO calls for this action. `fade` in ms.
    pub fn calls(&self, caps: &Caps, fade: u32) -> Vec<(&'static str, Value)> {
        let eff = |d: u32| if d < 30 { json!("sudden") } else { json!("smooth") };
        let d = fade.max(30);
        match self {
            Action::Power(on) => vec![("set_power", json!([if *on { "on" } else { "off" }, eff(fade), d]))],
            Action::Toggle => vec![("toggle", json!([]))],
            Action::Bright(b) => vec![("set_bright", json!([(*b).clamp(1, 100), eff(fade), d]))],
            Action::Ct(k) => vec![("set_ct_abx", json!([(*k).clamp(caps.ct_min, caps.ct_max), eff(fade), d]))],
            Action::Rgb(c) => vec![("set_rgb", json!([(*c).clamp(1, 0xffffff), eff(fade), d]))],
            Action::Hsv(h, s) => vec![("set_hsv", json!([(*h).min(359), (*s).min(100), eff(fade), d]))],
            Action::White(b, k) => vec![
                ("set_ct_abx", json!([(*k).clamp(caps.ct_min, caps.ct_max), eff(fade), d])),
                ("set_bright", json!([(*b).clamp(1, 100), eff(fade), d])),
            ],
            Action::Scene(name) => match scene(name) {
                Some(s) => vec![("start_cf", json!([s.count, s.end, s.flow]))],
                None => vec![],
            },
            Action::StopScene => vec![("stop_cf", json!([]))],
        }
    }

    /// Does the light have to be on for this?
    pub fn needs_power(&self) -> bool {
        !matches!(self, Action::Power(_) | Action::Toggle | Action::StopScene)
    }

    /// What the state will be after it, so that the panel follows at once.
    pub fn apply(&self, s: &mut LightState) {
        if self.needs_power() {
            s.power = true;
        }
        match self {
            Action::Power(on) => s.power = *on,
            Action::Toggle => s.power = !s.power,
            Action::Bright(b) => s.bright = *b,
            Action::Ct(k) => {
                s.ct = *k;
                s.mode = Mode::White;
            }
            Action::Rgb(c) => {
                s.rgb = *c;
                s.mode = Mode::Color;
            }
            Action::Hsv(h, sat) => {
                s.hue = *h;
                s.sat = *sat;
                s.mode = Mode::Hsv;
            }
            Action::White(b, k) => {
                s.bright = *b;
                s.ct = *k;
                s.mode = Mode::White;
            }
            Action::Scene(n) => {
                s.flowing = true;
                s.scene = Some(n.clone());
            }
            Action::StopScene => {
                s.flowing = false;
                s.scene = None;
            }
        }
        if !matches!(self, Action::Scene(_) | Action::Power(_) | Action::Toggle | Action::Bright(_)) {
            s.flowing = false;
            s.scene = None;
        }
    }
}

pub struct ColorPreset {
    pub name: &'static str,
    pub rgb: u32,
}

pub const COLORS: &[ColorPreset] = &[
    ColorPreset { name: "Red", rgb: 0xff0000 },
    ColorPreset { name: "Orange", rgb: 0xff6a00 },
    ColorPreset { name: "Amber", rgb: 0xffa000 },
    ColorPreset { name: "Yellow", rgb: 0xffe000 },
    ColorPreset { name: "Lime", rgb: 0x9dff00 },
    ColorPreset { name: "Green", rgb: 0x00ff30 },
    ColorPreset { name: "Mint", rgb: 0x00ffa0 },
    ColorPreset { name: "Cyan", rgb: 0x00e5ff },
    ColorPreset { name: "Sky", rgb: 0x0090ff },
    ColorPreset { name: "Blue", rgb: 0x0030ff },
    ColorPreset { name: "Indigo", rgb: 0x4b00ff },
    ColorPreset { name: "Violet", rgb: 0x9000ff },
    ColorPreset { name: "Magenta", rgb: 0xff00e0 },
    ColorPreset { name: "Pink", rgb: 0xff3090 },
    ColorPreset { name: "Rose", rgb: 0xff6080 },
    ColorPreset { name: "White", rgb: 0xffffff },
];

pub fn color_by_name(n: &str) -> Option<u32> {
    COLORS.iter().find(|c| c.name.eq_ignore_ascii_case(n)).map(|c| c.rgb)
}

pub struct WhitePreset {
    pub name: &'static str,
    pub icon: &'static str,
    pub bright: u8,
    pub ct: u16,
}

/// The desk lamp's modes in Mi Home, and the same idea for the others.
pub const WHITES: &[WhitePreset] = &[
    WhitePreset { name: "Reading", icon: "accessories-dictionary-symbolic", bright: 100, ct: 4000 },
    WhitePreset { name: "Computer", icon: "computer-symbolic", bright: 60, ct: 5000 },
    WhitePreset { name: "Focus", icon: "find-location-symbolic", bright: 100, ct: 6500 },
    WhitePreset { name: "Relax", icon: "weather-few-clouds-night-symbolic", bright: 50, ct: 2700 },
    WhitePreset { name: "Night", icon: "weather-clear-night-symbolic", bright: 1, ct: 2700 },
];

pub fn white_by_name(n: &str) -> Option<&'static WhitePreset> {
    WHITES.iter().find(|w| w.name.eq_ignore_ascii_case(n))
}

pub struct Scene {
    pub name: &'static str,
    /// 0 = forever
    pub count: u32,
    /// 0 = back to before, 1 = stay on the last step, 2 = turn off
    pub end: u32,
    /// steps of "duration ms, mode (1 colour, 2 white, 7 pause), value, brightness"
    pub flow: &'static str,
    /// for the button
    pub swatch: [u32; 3],
}

pub const SCENES: &[Scene] = &[
    Scene { name: "Candle", count: 0, end: 0, swatch: [0xff8a1c, 0xffb44a, 0xff7a10], flow: "800,2,2700,50,800,2,2700,30,1200,2,2700,80,800,2,2700,60,1200,2,2700,90,2400,2,2700,50,1200,2,2700,80,800,2,2700,60,400,2,2700,70" },
    Scene { name: "Sunrise", count: 1, end: 1, swatch: [0xff4400, 0xffa040, 0xfff0d0], flow: "50,1,16731392,1,360000,2,1700,10,540000,2,2700,100" },
    Scene { name: "Sunset", count: 1, end: 2, swatch: [0xffd090, 0xff8030, 0xa02000], flow: "50,2,2700,10,180000,2,1700,5,420000,1,16731136,1" },
    Scene { name: "Romance", count: 0, end: 0, swatch: [0x5915ed, 0xff1a6a, 0x8a00c0], flow: "4000,1,5838189,1,4000,1,6689834,1" },
    Scene { name: "Ocean", count: 0, end: 0, swatch: [0x0077ff, 0x00ffaa, 0x4b00ff], flow: "5000,1,30719,80,5000,1,65450,60,5000,1,4915455,80" },
    Scene { name: "Lava", count: 0, end: 0, swatch: [0xff0000, 0xff5000, 0xffa000], flow: "3000,1,16711680,100,3000,1,16732160,50,3000,1,16752640,100" },
    Scene { name: "Rainbow", count: 0, end: 0, swatch: [0xff0000, 0x00ff00, 0x0000ff], flow: "2000,1,16711680,100,2000,1,16776960,100,2000,1,65280,100,2000,1,65535,100,2000,1,255,100,2000,1,16711935,100" },
    Scene { name: "Party", count: 0, end: 0, swatch: [0xff00e0, 0x00e5ff, 0xffe000], flow: "300,1,16711904,100,300,1,58879,100,300,1,16769024,100,300,1,65328,100,300,1,4915455,100" },
    Scene { name: "Police", count: 0, end: 0, swatch: [0xff0000, 0x0030ff, 0xff0000], flow: "300,1,16711680,100,300,1,12543,100" },
    Scene { name: "Breathe", count: 0, end: 0, swatch: [0x0090ff, 0x003070, 0x0090ff], flow: "2000,1,37119,100,2000,1,37119,5" },
];

pub fn scene(name: &str) -> Option<&'static Scene> {
    SCENES.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// The running scene per light, between command-line calls (the lights only say "flowing").
fn scenes_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("lumen-scenes.json")
}

pub fn remember_scene(alias: &str, scene: Option<&str>) {
    let p = scenes_path();
    let mut m: serde_json::Map<String, Value> = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    match scene {
        Some(n) => m.insert(alias.to_string(), json!(n)),
        None => m.remove(alias),
    };
    let _ = std::fs::write(&p, Value::Object(m).to_string());
}

pub fn remembered_scene(alias: &str) -> Option<String> {
    let t = std::fs::read_to_string(scenes_path()).ok()?;
    let m: Value = serde_json::from_str(&t).ok()?;
    m[alias].as_str().map(String::from)
}

pub fn config_dir() -> PathBuf {
    directories::BaseDirs::new().map(|d| d.config_dir().join("lumen")).unwrap_or_else(|| PathBuf::from(".lumen"))
}

pub fn config_path() -> PathBuf {
    config_dir().join("lights.json")
}

pub fn load() -> Result<Vec<LightConfig>, String> {
    let p = config_path();
    let text = std::fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {e}\nimport your lights first: lumen import cloud.json", p.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))
}

/// Written readable by the user only: the tokens are the keys to the lights.
pub fn save(lights: &[LightConfig]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let p = config_path();
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.new");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(serde_json::to_string_pretty(lights).unwrap().as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().ok();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).ok();
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

/// Lights out of the token extractor's output (its -o file). Other devices are skipped.
pub fn import_cloud(v: &Value) -> Vec<LightConfig> {
    let mut out: Vec<LightConfig> = Vec::new();
    let servers = v.as_array().cloned().unwrap_or_default();
    for s in servers {
        for h in s["homes"].as_array().cloned().unwrap_or_default() {
            for d in h["devices"].as_array().cloned().unwrap_or_default() {
                let model = d["model"].as_str().unwrap_or("");
                if !model.starts_with("yeelink.light.") {
                    continue;
                }
                let (Some(ip), Some(token)) = (d["localip"].as_str().and_then(|s| s.parse().ok()), d["token"].as_str()) else { continue };
                let did = d["did"].as_str().map(String::from).unwrap_or_else(|| d["did"].to_string());
                if out.iter().any(|l| l.did == did) {
                    continue;
                }
                let kind = Caps::of(model).kind;
                let base = match kind {
                    Kind::Lamp => "desk",
                    Kind::Bulb => "bulb",
                    Kind::Strip => "strip",
                };
                let mut alias = base.to_string();
                let mut n = 2;
                while out.iter().any(|l| l.alias == alias) {
                    alias = format!("{base}{n}");
                    n += 1;
                }
                out.push(LightConfig { alias, name: d["name"].as_str().unwrap_or(model).to_string(), model: model.into(), ip, did, token: token.into(), mac: d["mac"].as_str().unwrap_or("").into() });
            }
        }
    }
    let order = |l: &LightConfig| match l.caps().kind {
        Kind::Lamp => 0,
        Kind::Bulb => 1,
        Kind::Strip => 2,
    };
    out.sort_by_key(|l| (order(l), l.alias.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_from_the_real_lights() {
        // what the desk lamp, the strip and the bulb answered on 2026-10-17
        let mut s = LightState::default();
        s.apply_props(&json!(["on", "80", "2600", "", "", "", "2", ""]));
        assert!(s.power);
        assert_eq!((s.bright, s.ct, s.mode), (80, 2600, Mode::White));
        s.apply_props(&json!(["on", "100", "4000", "13434624", "", "", "1", "0"]));
        assert_eq!((s.rgb, s.mode), (0xccff00, Mode::Color));
        s.apply_props(&json!(["off", "50", "3300", "16711680", "0", "100", "3", "1"]));
        assert!(!s.power && s.flowing);
        assert_eq!((s.hue, s.sat, s.mode), (0, 100, Mode::Hsv));
    }

    #[test]
    fn calls_clamp_to_the_model() {
        let lamp = Caps::of("yeelink.light.lamp4");
        assert!(!lamp.color);
        assert_eq!(Action::Ct(1700).calls(&lamp, 300), vec![("set_ct_abx", json!([2600, "smooth", 300]))]);
        assert_eq!(Action::Ct(6500).calls(&lamp, 300), vec![("set_ct_abx", json!([5000, "smooth", 300]))]);
        let bulb = Caps::of("yeelink.light.color1");
        assert!(bulb.color && bulb.scenes);
        assert_eq!(Action::Rgb(0).calls(&bulb, 0), vec![("set_rgb", json!([1, "sudden", 30]))]);
        assert_eq!(Action::Hsv(400, 120).calls(&bulb, 200), vec![("set_hsv", json!([359, 100, "smooth", 200]))]);
        assert_eq!(Caps::of("yeelink.light.strip1").kind, Kind::Strip);
    }

    #[test]
    fn scenes_are_well_formed() {
        for s in SCENES {
            let parts: Vec<&str> = s.flow.split(',').collect();
            assert_eq!(parts.len() % 4, 0, "{}", s.name);
            for step in parts.chunks(4) {
                let ms: u32 = step[0].parse().unwrap();
                assert!(ms >= 50, "{}: steps must be at least 50 ms", s.name);
                assert!(["1", "2", "7"].contains(&step[1]), "{}", s.name);
                let v: u32 = step[2].parse().unwrap();
                if step[1] == "2" {
                    assert!((1700..=6500).contains(&v), "{}", s.name);
                }
                if step[1] == "1" {
                    assert!(v <= 0xffffff, "{}", s.name);
                }
                let b: i32 = step[3].parse().unwrap();
                assert!((1..=100).contains(&b), "{}", s.name);
            }
        }
        assert!(Action::Scene("nope".into()).calls(&Caps::of("yeelink.light.color1"), 0).is_empty());
    }

    #[test]
    fn colours() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), (0, 0, 255));
        let (h, s, v) = rgb_to_hsv(0, 255, 0);
        assert_eq!((h.round(), s, v), (120.0, 1.0, 1.0));
        assert_eq!(split(join(1, 2, 3)), (1, 2, 3));
        let warm = kelvin_rgb(2700);
        let cool = kelvin_rgb(6500);
        assert!(warm.2 < cool.2 && warm.0 >= cool.0);
    }

    #[test]
    fn actions_update_state() {
        let mut s = LightState { power: false, flowing: true, scene: Some("Candle".into()), ..Default::default() };
        Action::Rgb(0x123456).apply(&mut s);
        assert!(s.power && !s.flowing && s.scene.is_none());
        assert_eq!(s.mode, Mode::Color);
        Action::Scene("Party".into()).apply(&mut s);
        Action::Bright(10).apply(&mut s);
        assert!(s.flowing, "brightness keeps a scene running");
        Action::Power(false).apply(&mut s);
        assert!(!s.power);
        assert_eq!(Action::Bright(3).slot(), 1);
        assert_eq!(Action::Hsv(1, 1).slot(), Action::Scene("x".into()).slot());
    }

    #[test]
    fn imports_lights_only() {
        let v = json!([{ "server": "cn", "homes": [{ "devices": [
            { "name": "Desk lamp", "did": "123", "model": "yeelink.light.lamp4", "localip": "192.0.2.10", "token": "00112233445566778899aabbccddeeff", "mac": "x" },
            { "name": "Video camera", "did": "9", "model": "isa.camera.df3", "localip": "192.0.2.11", "token": "00112233445566778899aabbccddeeff" },
            { "name": "Strip", "did": "77", "model": "yeelink.light.strip1", "localip": "192.0.2.12", "token": "00112233445566778899aabbccddeeff" },
            { "name": "Light bulb", "did": "55", "model": "yeelink.light.color1", "localip": "192.0.2.13", "token": "00112233445566778899aabbccddeeff" }
        ]}]}]);
        let l = import_cloud(&v);
        let aliases: Vec<&str> = l.iter().map(|x| x.alias.as_str()).collect();
        assert_eq!(aliases, ["desk", "bulb", "strip"]);
        assert_eq!(l[0].device_id(), Some(123));
    }
}
