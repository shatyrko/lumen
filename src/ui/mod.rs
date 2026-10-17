//! The application: tray icon and panel.

pub mod panel;
pub mod tray;
pub mod widgets;

use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio};

use crate::hub::{Hub, HubEvent};
use crate::ipc::{self, UiMsg};

pub fn run() -> ExitCode {
    if ipc::running() {
        eprintln!("Lumen is already running; opening its panel.");
        let _ = ipc::send("Toggle");
        return ExitCode::SUCCESS;
    }
    let cfgs = match crate::lights::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lumen: {e}");
            return ExitCode::FAILURE;
        }
    };
    let app = adw::Application::builder().application_id(ipc::NAME).flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_startup(|_| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&format!("{}\n{}", include_str!("style.css"), panel::swatch_css()));
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
    let started = Rc::new(Cell::new(false));
    let cfgs = Rc::new(std::cell::RefCell::new(Some(cfgs)));
    let code = Rc::new(Cell::new(ExitCode::SUCCESS));
    {
        let code = code.clone();
        app.connect_activate(move |app| {
            if started.replace(true) {
                return;
            }
            let Some(c) = cfgs.borrow_mut().take() else { return };
            if let Err(e) = setup(app, c) {
                eprintln!("lumen: {e}");
                code.set(ExitCode::FAILURE);
                app.quit();
            }
        });
    }
    app.run_with_args::<&str>(&[]);
    code.get()
}

fn setup(app: &adw::Application, cfgs: Vec<crate::lights::LightConfig>) -> Result<(), String> {
    let (ev_tx, ev_rx) = async_channel::unbounded::<HubEvent>();
    let hub = Hub::start(cfgs, Some(ev_tx), true);
    let (ui_tx, ui_rx) = async_channel::unbounded::<UiMsg>();
    let conn = ipc::serve(ui_tx.clone()).map_err(|e| format!("cannot offer the D-Bus service (is Lumen already running?): {e}"))?;
    std::mem::forget(conn);

    let gtk_app: &gtk::Application = app.upcast_ref();
    let panel = panel::Panel::new(gtk_app, &hub);
    {
        let (hub, app) = (hub.clone(), app.clone());
        panel.connect_quit(move || {
            hub.shutdown();
            app.quit();
        });
    }
    let tray = Rc::new(tray::spawn(tray::Tray { ui: ui_tx.clone(), hub: hub.clone(), views: hub.views() }));
    if tray.is_none() {
        eprintln!("lumen: no tray available; open the panel with `lumen panel`");
    }
    hub.refresh();

    // the tray, the command line and D-Bus
    {
        let (panel, hub, app) = (panel.clone(), hub.clone(), app.clone());
        glib::spawn_future_local(async move {
            while let Ok(msg) = ui_rx.recv().await {
                match msg {
                    UiMsg::Toggle => panel.toggle(),
                    UiMsg::Show => panel.show(),
                    UiMsg::Hide => panel.hide(),
                    UiMsg::Refresh => hub.refresh(),
                    UiMsg::Quit => {
                        hub.shutdown();
                        app.quit();
                    }
                }
            }
        });
    }

    // the lights changed: redraw a moment later, once
    {
        let (panel, hub, tray) = (panel.clone(), hub.clone(), tray.clone());
        let pending = Rc::new(Cell::new(false));
        let first = Rc::new(Cell::new(true));
        glib::spawn_future_local(async move {
            while let Ok(ev) = ev_rx.recv().await {
                match ev {
                    HubEvent::Changed => {
                        if pending.replace(true) {
                            continue;
                        }
                        let (panel, hub, tray, pending, first) = (panel.clone(), hub.clone(), tray.clone(), pending.clone(), first.clone());
                        glib::timeout_add_local_once(Duration::from_millis(30), move || {
                            pending.set(false);
                            let views = hub.views();
                            if first.get() && views.iter().all(|v| v.state.online.is_some()) {
                                first.set(false);
                                panel.first_answers();
                            }
                            if panel.is_visible() {
                                panel.sync(&views);
                            }
                            if let Some(t) = tray.as_ref() {
                                t.update(|t| t.views = views.clone());
                            }
                        });
                    }
                    HubEvent::Notice(m) => panel.toast(&m, false),
                }
            }
        });
    }

    // read the lights now and then: every 3 s while the panel is open, else every 30 s
    // (changes from the Mi Home app or a wall switch)
    {
        let (panel, hub) = (panel.clone(), hub.clone());
        let tick = Rc::new(Cell::new(0u32));
        glib::timeout_add_local(Duration::from_secs(3), move || {
            let n = tick.get() + 1;
            tick.set(n);
            if panel.is_visible() || n % 10 == 0 {
                hub.refresh();
            }
            glib::ControlFlow::Continue
        });
    }

    std::mem::forget(app.hold());
    Ok(())
}
