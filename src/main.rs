use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use lumen::hub::{Hub, FADE};
use lumen::lights::{self, Action};
use lumen::{ipc, ui};

#[derive(Parser)]
#[command(name = "lumen", version, about = "Your Xiaomi/Yeelight lights over the local network, no cloud. Run without a command to start the tray app.")]
struct Cli {
    /// Print the lights' state as JSON (for the Noctalia bar widget)
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the tray app (the default)
    Run,
    /// Open or close the panel of the running app (for a keyboard shortcut)
    Panel,
    /// Quit the running app
    Quit,
    /// Show the state of the lights
    Status { light: Option<String> },
    /// Switch on. LIGHT: desk, bulb, strip, a comma list or all (the default)
    On { light: Option<String> },
    Off { light: Option<String> },
    Toggle { light: Option<String> },
    /// Brightness 1-100, or +10 / -10
    #[command(allow_negative_numbers = true)]
    Bright { value: String, light: Option<String> },
    /// White colour temperature in kelvin (desk lamp 2600-5000, bulb and strip 1700-6500)
    Temp { kelvin: u16, light: Option<String> },
    /// A colour: a name (red, orange, amber, yellow, lime, green, mint, cyan, sky, blue,
    /// indigo, violet, magenta, pink, rose, white) or #RRGGBB
    Color { color: String, light: Option<String> },
    /// Any colour by hue (0-359) and saturation (0-100)
    Hsv { hue: u16, sat: u8, light: Option<String> },
    /// White at a brightness and colour temperature together
    White { bright: u8, kelvin: u16, light: Option<String> },
    /// White presets: reading, computer, focus, relax, night
    Preset { name: String, light: Option<String> },
    /// Scenes: candle, sunrise, sunset, romance, ocean, lava, rainbow, party, police, breathe; or stop
    Scene { name: String, light: Option<String> },
    /// Look for the lights on the network again (after the router gave them new addresses)
    Search,
    /// Take the lights out of the token extractor's output file (its -o file)
    Import { file: String },
    /// Start with the session (on) or not (off)
    Autostart { state: String },
    /// Put a launcher entry and the icon into your applications menu
    InstallLauncher,
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).format_timestamp_millis().init();
    let cli = Cli::parse();
    let json = cli.json;
    let command = |light: Option<String>, make: &dyn Fn(&lumen::hub::View) -> Result<Action, String>| command(light, json, false, make);
    match cli.command.unwrap_or(Cmd::Run) {
        Cmd::Run => ui::run(),
        Cmd::Panel => match ipc::send("Toggle") {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => fail("the app is not running; start it with `lumen`"),
        },
        Cmd::Quit => match ipc::send("Quit") {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => fail("the app is not running"),
        },
        Cmd::Import { file } => import(&file),
        Cmd::Autostart { state } => match lumen::autostart::set(state == "on") {
            Ok(p) if state == "on" => {
                println!("Lumen starts with the session ({})", p.display());
                ExitCode::SUCCESS
            }
            Ok(_) => {
                println!("Lumen no longer starts with the session");
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        },
        Cmd::InstallLauncher => match lumen::autostart::install_launcher() {
            Ok(p) => {
                println!("installed {}", p.display());
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e),
        },
        Cmd::Search => {
            let Some(hub) = start() else { return ExitCode::FAILURE };
            let moved = hub.search(true);
            hub.refresh();
            hub.wait_idle(Duration::from_secs(10));
            print_status(&hub, None, json);
            if !json {
                println!("{moved} light(s) had a new address");
            }
            ExitCode::SUCCESS
        }
        Cmd::Status { light } => {
            let Some(hub) = start() else { return ExitCode::FAILURE };
            hub.refresh();
            hub.wait_idle(Duration::from_secs(10));
            print_status(&hub, light.as_deref(), json)
        }
        Cmd::On { light } => command(light, &|_| Ok(Action::Power(true))),
        Cmd::Off { light } => command(light, &|_| Ok(Action::Power(false))),
        Cmd::Toggle { light } => command(light, &|_| Ok(Action::Toggle)),
        Cmd::Hsv { hue, sat, light } => command(light, &|v| if v.cfg.caps().color { Ok(Action::Hsv(hue % 360, sat.min(100))) } else { Err("has no colour".into()) }),
        Cmd::White { bright, kelvin, light } => command(light, &|_| Ok(Action::White(bright.clamp(1, 100), kelvin))),
        Cmd::Bright { value, light } => {
            let rel = value.starts_with('+') || value.starts_with('-');
            let n: i32 = match value.trim_start_matches('+').parse() {
                Ok(n) => n,
                Err(_) => return fail(&format!("not a brightness: {value}")),
            };
            // only a relative change needs to read the lights first
            self::command(light, json, rel, &move |v| {
                let b = if rel { v.state.bright as i32 + n } else { n };
                Ok(Action::Bright(b.clamp(1, 100) as u8))
            })
        }
        Cmd::Temp { kelvin, light } => command(light, &move |_| Ok(Action::Ct(kelvin))),
        Cmd::Color { color, light } => {
            let rgb = lights::color_by_name(&color).or_else(|| ui::panel::parse_hex(&color));
            let Some(rgb) = rgb else {
                return fail(&format!("not a colour: {color} (a name like blue, or #RRGGBB)"));
            };
            command(light, &move |v| if v.cfg.caps().color { Ok(Action::Rgb(rgb)) } else { Err("has no colour".into()) })
        }
        Cmd::Preset { name, light } => {
            let Some(w) = lights::white_by_name(&name) else {
                return fail(&format!("no preset {name} (have: {})", lights::WHITES.iter().map(|w| w.name.to_lowercase()).collect::<Vec<_>>().join(", ")));
            };
            command(light, &move |_| Ok(Action::White(w.bright, w.ct)))
        }
        Cmd::Scene { name, light } => {
            if name == "stop" {
                return command(light, &|_| Ok(Action::StopScene));
            }
            let Some(s) = lights::scene(&name) else {
                return fail(&format!("no scene {name} (have: {}, stop)", lights::SCENES.iter().map(|s| s.name.to_lowercase()).collect::<Vec<_>>().join(", ")));
            };
            command(light, &move |v| if v.cfg.caps().scenes { Ok(Action::Scene(s.name.into())) } else { Err("has no scenes".into()) })
        }
    }
}

fn fail(m: &str) -> ExitCode {
    eprintln!("lumen: {m}");
    ExitCode::FAILURE
}

fn start() -> Option<Hub> {
    match lights::load() {
        Ok(c) => Some(Hub::start(c, None, true)),
        Err(e) => {
            eprintln!("lumen: {e}");
            None
        }
    }
}

/// Run one action on the chosen lights, then print where they are.
fn command(light: Option<String>, json: bool, needs_state: bool, make: &dyn Fn(&lumen::hub::View) -> Result<Action, String>) -> ExitCode {
    let Some(hub) = start() else { return ExitCode::FAILURE };
    let targets = match hub.resolve(light.as_deref().unwrap_or("all")) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    if needs_state {
        hub.refresh();
        hub.wait_idle(Duration::from_secs(10));
    }
    for v in hub.views().iter().filter(|v| targets.contains(&v.cfg.alias)) {
        match make(v) {
            Ok(a) => {
                // the lights do not say which scene runs; remember the one we started
                match &a {
                    Action::Scene(n) => lights::remember_scene(&v.cfg.alias, Some(n)),
                    a if a.slot() == 2 => lights::remember_scene(&v.cfg.alias, None),
                    _ => {}
                }
                hub.act(&v.cfg.alias, a, FADE)
            }
            Err(e) if !json => println!("{:6} {e}, skipped", v.cfg.alias),
            Err(_) => {}
        }
    }
    hub.wait_idle(Duration::from_secs(10));
    // print what the lights say, not what we asked for
    hub.refresh();
    hub.wait_idle(Duration::from_secs(10));
    if ipc::running() {
        let _ = ipc::send("Refresh");
    }
    print_status(&hub, light.as_deref(), json)
}

fn print_status(hub: &Hub, light: Option<&str>, json: bool) -> ExitCode {
    let targets = match hub.resolve(light.unwrap_or("all")) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let mut ok = true;
    let mut rows = Vec::new();
    for mut v in hub.views().into_iter().filter(|v| targets.contains(&v.cfg.alias)) {
        if v.state.flowing && v.state.scene.is_none() {
            v.state.scene = lights::remembered_scene(&v.cfg.alias);
        }
        ok &= v.state.online == Some(true);
        if json {
            rows.push(view_json(&v));
            continue;
        }
        let line = ui::panel::describe(&v);
        if v.state.online != Some(true) {
            println!("{:6} {line} ({})", v.cfg.alias, v.state.error.as_deref().unwrap_or("no answer"));
        } else {
            println!("{:6} {line}", v.cfg.alias);
        }
    }
    if json {
        println!("{}", serde_json::Value::Array(rows));
    }
    hub.shutdown();
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// One light for the bar widget: what it is, what it can do, what it does now.
fn view_json(v: &lumen::hub::View) -> serde_json::Value {
    let s = &v.state;
    let caps = v.cfg.caps();
    let (r, g, b) = s.shown_rgb();
    serde_json::json!({
        "alias": v.cfg.alias,
        "name": v.cfg.name,
        "model": v.cfg.model,
        "ip": v.cfg.ip.to_string(),
        "online": s.online == Some(true),
        "error": s.error,
        "power": s.power,
        "bright": s.bright,
        "ct": s.ct,
        "rgb": format!("#{:06x}", s.rgb),
        "hue": s.hue,
        "sat": s.sat,
        "mode": match s.mode { lights::Mode::Color => "color", lights::Mode::Hsv => "hsv", lights::Mode::White => "white" },
        "flowing": s.flowing,
        "scene": s.scene,
        "shown": format!("#{:06x}", lights::join(r, g, b)),
        "describe": ui::panel::describe(v),
        "can_color": caps.color,
        "can_scenes": caps.scenes,
        "ct_min": caps.ct_min,
        "ct_max": caps.ct_max,
    })
}

fn import(file: &str) -> ExitCode {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => return fail(&format!("{file}: {e}")),
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return fail(&format!("{file}: {e}")),
    };
    let found = lights::import_cloud(&v);
    if found.is_empty() {
        return fail("no Yeelight lights with a token and an address in that file");
    }
    if let Err(e) = lights::save(&found) {
        return fail(&e);
    }
    for l in &found {
        println!("{:6} {:<28} {:<24} {}", l.alias, l.name, l.model, l.ip);
    }
    println!("saved to {} (readable by you only)", lights::config_path().display());
    ExitCode::SUCCESS
}
