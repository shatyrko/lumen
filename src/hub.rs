//! The lights at run time: one worker thread per light, so a slow or unreachable light never
//! holds up the others or the panel. Actions update the shown state at once and are sent in
//! the background; a dragged slider's values replace each other while they wait.

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::lights::{self, Action, LightConfig, LightState, PROPS};
use crate::miio::{self, Client, Error};

#[derive(Debug, Clone)]
pub enum HubEvent {
    Changed,
    Notice(String),
}

enum Job {
    Act(Action, u32),
    Refresh,
}

pub struct Light {
    pub cfg: Mutex<LightConfig>,
    pub state: Mutex<LightState>,
    jobs: Mutex<Vec<Job>>,
    cv: Condvar,
}

struct Inner {
    lights: Vec<Arc<Light>>,
    events: Option<async_channel::Sender<HubEvent>>,
    pending: AtomicUsize,
    quit: AtomicBool,
    last_search: Mutex<Option<Instant>>,
    /// write the config file when an address changed (not in tests)
    persist: bool,
}

#[derive(Clone)]
pub struct Hub(Arc<Inner>);

#[derive(Debug, Clone)]
pub struct View {
    pub cfg: LightConfig,
    pub state: LightState,
}

/// Fade for buttons; sliders use a shorter one so that the light keeps up with the hand.
pub const FADE: u32 = 300;
pub const FADE_DRAG: u32 = 150;

impl Hub {
    pub fn start(cfgs: Vec<LightConfig>, events: Option<async_channel::Sender<HubEvent>>, persist: bool) -> Hub {
        let lights: Vec<Arc<Light>> = cfgs
            .into_iter()
            .map(|c| Arc::new(Light { cfg: Mutex::new(c), state: Mutex::new(LightState::default()), jobs: Mutex::new(Vec::new()), cv: Condvar::new() }))
            .collect();
        let hub = Hub(Arc::new(Inner { lights, events, pending: AtomicUsize::new(0), quit: AtomicBool::new(false), last_search: Mutex::new(None), persist }));
        for (i, _) in hub.0.lights.iter().enumerate() {
            let h = hub.clone();
            std::thread::Builder::new().name(format!("light-{i}")).spawn(move || h.worker(i)).expect("thread");
        }
        hub
    }

    pub fn views(&self) -> Vec<View> {
        self.0.lights.iter().map(|l| View { cfg: l.cfg.lock().unwrap().clone(), state: l.state.lock().unwrap().clone() }).collect()
    }

    pub fn aliases(&self) -> Vec<String> {
        self.0.lights.iter().map(|l| l.cfg.lock().unwrap().alias.clone()).collect()
    }

    fn index(&self, alias: &str) -> Option<usize> {
        self.0.lights.iter().position(|l| l.cfg.lock().unwrap().alias == alias)
    }

    /// "all", one alias, or a comma list.
    pub fn resolve(&self, target: &str) -> Result<Vec<String>, String> {
        let all = self.aliases();
        if target.is_empty() || target == "all" {
            return Ok(all);
        }
        let mut out = Vec::new();
        for t in target.split(',') {
            if !all.iter().any(|a| a == t) {
                return Err(format!("unknown light: {t} (have: {})", all.join(", ")));
            }
            out.push(t.to_string());
        }
        Ok(out)
    }

    fn emit(&self, e: HubEvent) {
        if let Some(tx) = &self.0.events {
            let _ = tx.try_send(e);
        }
    }

    fn push(&self, i: usize, job: Job) {
        let l = &self.0.lights[i];
        self.0.pending.fetch_add(1, Ordering::SeqCst);
        l.jobs.lock().unwrap().push(job);
        l.cv.notify_one();
    }

    pub fn act(&self, alias: &str, a: Action, fade: u32) {
        let Some(i) = self.index(alias) else { return };
        a.apply(&mut self.0.lights[i].state.lock().unwrap());
        self.emit(HubEvent::Changed);
        self.push(i, Job::Act(a, fade));
    }

    pub fn act_all(&self, a: Action, fade: u32) {
        for alias in self.aliases() {
            self.act(&alias, a.clone(), fade);
        }
    }

    pub fn refresh(&self) {
        for i in 0..self.0.lights.len() {
            // one refresh waiting is enough
            if !self.0.lights[i].jobs.lock().unwrap().iter().any(|j| matches!(j, Job::Refresh)) {
                self.push(i, Job::Refresh);
            }
        }
    }

    /// For the command line: wait until everything queued has been sent.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let until = Instant::now() + timeout;
        while self.0.pending.load(Ordering::SeqCst) > 0 {
            if Instant::now() > until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }

    pub fn shutdown(&self) {
        self.0.quit.store(true, Ordering::SeqCst);
        for l in &self.0.lights {
            l.cv.notify_all();
        }
    }

    /// Look for every light on the network by its device id; move the ones whose address
    /// changed. At most once every 20 s unless forced.
    pub fn search(&self, force: bool) -> usize {
        {
            let mut last = self.0.last_search.lock().unwrap();
            if !force && last.map(|t| t.elapsed() < Duration::from_secs(20)).unwrap_or(false) {
                return 0;
            }
            *last = Some(Instant::now());
        }
        let near = self.0.lights.first().map(|l| l.cfg.lock().unwrap().ip).unwrap_or(Ipv4Addr::new(192, 168, 1, 1));
        let t0 = Instant::now();
        let want: Vec<u32> = self.0.lights.iter().filter_map(|l| l.cfg.lock().unwrap().device_id()).collect();
        let found = miio::discover(near, Duration::from_millis(1000), &want);
        log::debug!("search: {} device(s) in {} ms", found.len(), t0.elapsed().as_millis());
        let mut moved = 0;
        for l in &self.0.lights {
            let mut c = l.cfg.lock().unwrap();
            let Some(did) = c.device_id() else { continue };
            if let Some((_, ip)) = found.iter().find(|(d, _)| *d == did) {
                if *ip != c.ip {
                    log::info!("{} moved from {} to {}", c.alias, c.ip, ip);
                    c.ip = *ip;
                    moved += 1;
                }
            }
        }
        if moved > 0 {
            if self.0.persist {
                let all: Vec<LightConfig> = self.0.lights.iter().map(|l| l.cfg.lock().unwrap().clone()).collect();
                if let Err(e) = lights::save(&all) {
                    log::warn!("cannot save the new addresses: {e}");
                }
            }
            self.emit(HubEvent::Notice(format!("Found {moved} light(s) at a new address")));
        }
        moved
    }

    fn worker(&self, i: usize) {
        let l = self.0.lights[i].clone();
        let (ip, token) = {
            let c = l.cfg.lock().unwrap();
            (c.ip, miio::parse_token(&c.token))
        };
        let Some(token) = token else {
            let mut s = l.state.lock().unwrap();
            s.online = Some(false);
            s.error = Some("the token in lights.json is not 32 hex digits".into());
            return;
        };
        let mut client = match Client::new(ip, token) {
            Ok(c) => c,
            Err(e) => {
                l.state.lock().unwrap().error = Some(e.to_string());
                return;
            }
        };
        let mut last_sent = Instant::now() - Duration::from_secs(1);
        // what this light last told us about power (the shared state is optimistic and runs ahead)
        let mut known_on: Option<bool> = None;
        loop {
            let jobs = {
                let mut q = l.jobs.lock().unwrap();
                while q.is_empty() && !self.0.quit.load(Ordering::SeqCst) {
                    q = l.cv.wait(q).unwrap();
                }
                if self.0.quit.load(Ordering::SeqCst) {
                    return;
                }
                std::mem::take(&mut *q)
            };
            let n = jobs.len();
            let (acts, refresh) = coalesce(jobs);
            // a light takes a command about every 50 ms without dropping any
            let gap = Duration::from_millis(60);
            if last_sent.elapsed() < gap {
                std::thread::sleep(gap - last_sent.elapsed());
            }
            let caps = l.cfg.lock().unwrap().caps();
            let cfg_ip = l.cfg.lock().unwrap().ip;
            if client.addr.ip() != std::net::IpAddr::V4(cfg_ip) {
                client.set_ip(cfg_ip);
            }
            let mut result: Result<(), Error> = Ok(());
            let mut want_refresh = refresh;
            for (a, fade) in &acts {
                let mut calls = Vec::new();
                // Yeelights ignore colour and brightness while off: switch on first,
                // unless this worker knows it is on.
                if a.needs_power() && known_on != Some(true) {
                    calls.push(("set_power", serde_json::json!(["on", "smooth", *fade])));
                }
                calls.extend(a.calls(&caps, *fade));
                for (m, p) in calls {
                    result = self.send(i, &mut client, m, p.clone()).map(|_| ());
                    if result.is_err() {
                        break;
                    }
                    match m {
                        "set_power" => known_on = Some(p[0] == "on"),
                        "toggle" => known_on = None,
                        _ => {}
                    }
                }
                if result.is_err() {
                    break;
                }
                if matches!(a, Action::Toggle | Action::Scene(_) | Action::StopScene) {
                    want_refresh = true;
                }
            }
            last_sent = Instant::now();
            if result.is_ok() && want_refresh {
                result = self.send(i, &mut client, "get_prop", serde_json::json!(PROPS)).map(|v| {
                    let mut s = l.state.lock().unwrap();
                    let scene = s.scene.clone();
                    s.apply_props(&v);
                    known_on = Some(s.power);
                    if s.flowing && s.scene.is_none() {
                        s.scene = scene;
                    }
                });
            }
            {
                let mut s = l.state.lock().unwrap();
                match &result {
                    Ok(()) => {
                        s.online = Some(true);
                        s.error = None;
                    }
                    Err(e) => {
                        s.online = Some(false);
                        s.error = Some(e.to_string());
                    }
                }
            }
            self.emit(HubEvent::Changed);
            self.0.pending.fetch_sub(n, Ordering::SeqCst);
        }
    }

    /// One call; when the light does not answer, look for it at another address and try once more.
    fn send(&self, i: usize, client: &mut Client, method: &str, params: serde_json::Value) -> Result<serde_json::Value, Error> {
        let t0 = Instant::now();
        let r = client.call(method, params.clone());
        log::debug!("light {i}: {method} -> {} in {} ms", if r.is_ok() { "ok" } else { "failed" }, t0.elapsed().as_millis());
        let r = match r {
            Err(Error::Timeout) => {
                if self.search(false) > 0 {
                    let ip = self.0.lights[i].cfg.lock().unwrap().ip;
                    client.set_ip(ip);
                    client.call(method, params)
                } else {
                    Err(Error::Timeout)
                }
            }
            other => other,
        };
        r
    }
}

/// Keep, per slot, only the newest action; keep the order of those newest ones.
fn coalesce(jobs: Vec<Job>) -> (Vec<(Action, u32)>, bool) {
    let mut refresh = false;
    let mut acts: Vec<(Action, u32)> = Vec::new();
    for j in jobs {
        match j {
            Job::Refresh => refresh = true,
            Job::Act(a, f) => {
                // Toggle cannot be merged: two toggles cancel out.
                if !matches!(a, Action::Toggle) {
                    acts.retain(|(b, _)| b.slot() != a.slot() || matches!(b, Action::Toggle));
                }
                if matches!(a, Action::Power(_)) {
                    acts.retain(|(b, _)| !matches!(b, Action::Toggle));
                }
                acts.push((a, f));
            }
        }
    }
    (acts, refresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_values_collapse_to_the_last() {
        let jobs = vec![
            Job::Act(Action::Bright(10), 150),
            Job::Act(Action::Rgb(1), 150),
            Job::Act(Action::Bright(20), 150),
            Job::Refresh,
            Job::Act(Action::Hsv(5, 5), 150),
            Job::Act(Action::Bright(30), 150),
        ];
        let (acts, refresh) = coalesce(jobs);
        assert!(refresh);
        assert_eq!(acts.iter().map(|a| a.0.clone()).collect::<Vec<_>>(), vec![Action::Hsv(5, 5), Action::Bright(30)]);
    }

    #[test]
    fn power_wins_over_toggles() {
        let (acts, _) = coalesce(vec![Job::Act(Action::Toggle, 0), Job::Act(Action::Toggle, 0)]);
        assert_eq!(acts.len(), 2);
        let (acts, _) = coalesce(vec![Job::Act(Action::Toggle, 0), Job::Act(Action::Power(false), 0)]);
        assert_eq!(acts.iter().map(|a| a.0.clone()).collect::<Vec<_>>(), vec![Action::Power(false)]);
    }

    #[test]
    fn unknown_targets_are_refused() {
        let cfg = LightConfig { alias: "desk".into(), name: "Desk".into(), model: "yeelink.light.lamp4".into(), ip: Ipv4Addr::new(127, 0, 0, 1), did: "1".into(), token: "00".repeat(16), mac: String::new() };
        let hub = Hub::start(vec![cfg], None, false);
        assert_eq!(hub.resolve("all").unwrap(), vec!["desk"]);
        assert!(hub.resolve("desk,lamp").is_err());
        hub.shutdown();
    }
}
