//! The panel that drops down from the tray icon: one card per light with its switch and
//! brightness, and the rest of its controls (colour wheel, presets, white temperature,
//! scenes) folded underneath, one card open at a time.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use super::widgets::*;
use crate::hub::{Hub, View, FADE, FADE_DRAG};
use crate::lights::{self, Action, Caps, Mode};

struct Card {
    alias: String,
    root: gtk::Box,
    dot: Dot,
    title: gtk::Label,
    sub: gtk::Label,
    switch: gtk::Switch,
    switch_guard: Rc<Cell<bool>>,
    bright: SliderRow,
    chevron: gtk::Image,
    details: gtk::Revealer,
    stack: gtk::Stack,
    tabs: Vec<(String, gtk::ToggleButton)>,
    tab_guard: Rc<Cell<bool>>,
    ct: SliderRow,
    wheel: Option<Wheel>,
    hex: Option<(gtk::Entry, Rc<Cell<bool>>)>,
    swatches: Vec<(u32, gtk::Button)>,
    scenes: Vec<(String, gtk::Button)>,
    stop: Option<gtk::Button>,
    retry: gtk::Button,
    body: gtk::Box,
}

pub struct Panel {
    pub window: gtk::Window,
    scrim: gtk::Window,
    hub: Hub,
    cards: RefCell<Vec<Card>>,
    list: gtk::Box,
    toast: gtk::Label,
    toast_gen: Rc<Cell<u32>>,
    open: RefCell<Option<String>>,
    on_quit: RefCell<Option<Box<dyn Fn()>>>,
    autostart: gtk::CheckButton,
}

impl Panel {
    pub fn new(app: &gtk::Application, hub: &Hub) -> Rc<Panel> {
        let window = gtk::Window::builder().application(app).title("Lumen").decorated(false).resizable(false).build();
        window.add_css_class("lm-window");
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("lumen-panel"));
        window.set_keyboard_mode(KeyboardMode::OnDemand);
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Top, 6);
        window.set_margin(Edge::Right, 6);

        let scrim = gtk::Window::builder().application(app).title("Lumen scrim").decorated(false).build();
        scrim.add_css_class("lm-scrim");
        scrim.init_layer_shell();
        scrim.set_layer(Layer::Overlay);
        scrim.set_namespace(Some("lumen-scrim"));
        scrim.set_keyboard_mode(KeyboardMode::None);
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            scrim.set_anchor(e, true);
        }
        scrim.set_exclusive_zone(0);

        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        outer.add_css_class("lm-panel");
        outer.set_margin_top(6);
        outer.set_margin_bottom(18);
        outer.set_margin_start(18);
        outer.set_margin_end(12);

        // header: title, all on, all off, menu
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        header.add_css_class("lm-header");
        let title = gtk::Label::new(Some("Lights"));
        title.add_css_class("lm-heading");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        let all_on = gtk::Button::with_label("All on");
        all_on.add_css_class("lm-pill");
        let all_off = gtk::Button::with_label("All off");
        all_off.add_css_class("lm-pill");
        let more = gtk::MenuButton::new();
        more.set_icon_name("view-more-symbolic");
        more.add_css_class("flat");
        more.add_css_class("circular");
        let pop = gtk::Popover::new();
        pop.add_css_class("lm-menu");
        let more_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        pop.set_child(Some(&more_box));
        more.set_popover(Some(&pop));
        header.append(&title);
        header.append(&all_on);
        header.append(&all_off);
        header.append(&more);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
        list.set_margin_start(12);
        list.set_margin_end(12);
        list.set_margin_bottom(12);

        let toast = gtk::Label::new(None);
        toast.add_css_class("lm-toast");
        toast.set_wrap(true);
        toast.set_visible(false);
        toast.set_margin_start(12);
        toast.set_margin_end(12);
        toast.set_margin_bottom(10);

        outer.append(&header);
        outer.append(&list);
        outer.append(&toast);
        window.set_child(Some(&outer));

        let autostart = gtk::CheckButton::with_label("Start with the session");
        autostart.set_active(crate::autostart::is_enabled());

        let panel = Rc::new(Panel {
            window,
            scrim,
            hub: hub.clone(),
            cards: RefCell::new(Vec::new()),
            list,
            toast,
            toast_gen: Rc::new(Cell::new(0)),
            open: RefCell::new(None),
            on_quit: RefCell::new(None),
            autostart,
        });

        {
            let hub = hub.clone();
            all_on.connect_clicked(move |_| hub.act_all(Action::Power(true), FADE));
        }
        {
            let hub = hub.clone();
            all_off.connect_clicked(move |_| hub.act_all(Action::Power(false), FADE));
        }
        panel.fill_menu(&more_box, &pop);

        // closing: Esc, a click outside, focus lost
        {
            let p = panel.clone();
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    p.hide();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            panel.window.add_controller(keys);
        }
        {
            let p = panel.clone();
            let click = gtk::GestureClick::new();
            click.connect_pressed(move |_, _, _, _| p.hide());
            panel.scrim.add_controller(click);
        }
        {
            let p = panel.clone();
            panel.window.connect_is_active_notify(move |w| {
                if !w.is_active() {
                    let p = p.clone();
                    glib::timeout_add_local_once(Duration::from_millis(350), move || {
                        if p.window.is_visible() && !p.window.is_active() && !popover_open(&p.window) {
                            p.hide();
                        }
                    });
                }
            });
        }

        for v in hub.views() {
            let c = panel.make_card(&v);
            panel.list.append(&c.root);
            panel.cards.borrow_mut().push(c);
        }
        // the first card starts open (LUMEN_OPEN=bulb picks another, for testing)
        let start = std::env::var("LUMEN_OPEN").ok().or_else(|| panel.cards.borrow().first().map(|c| c.alias.clone()));
        *panel.open.borrow_mut() = start;
        panel
    }

    fn fill_menu(self: &Rc<Self>, b: &gtk::Box, pop: &gtk::Popover) {
        let add = |label: &str, f: Box<dyn Fn()>| {
            let btn = gtk::Button::with_label(label);
            btn.add_css_class("flat");
            if let Some(l) = btn.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                l.set_xalign(0.0);
            }
            let pop = pop.clone();
            btn.connect_clicked(move |_| {
                pop.popdown();
                f();
            });
            b.append(&btn);
        };
        let p = self.clone();
        add("Find lights again", Box::new(move || {
            p.toast("Looking for the lights on the network…", false);
            let hub = p.hub.clone();
            std::thread::spawn(move || {
                hub.search(true);
                hub.refresh();
            });
        }));
        let p = self.clone();
        add("Read the lights again", Box::new(move || p.hub.refresh()));
        b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        {
            let p = self.clone();
            self.autostart.connect_toggled(move |c| {
                if let Err(e) = crate::autostart::set(c.is_active()) {
                    p.toast(&e, true);
                }
            });
        }
        b.append(&self.autostart);
        b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let p = self.clone();
        add("Quit Lumen", Box::new(move || {
            if let Some(f) = p.on_quit.borrow().as_ref() {
                f();
            }
        }));
    }

    pub fn connect_quit(&self, f: impl Fn() + 'static) {
        *self.on_quit.borrow_mut() = Some(Box::new(f));
    }

    fn make_card(self: &Rc<Self>, v: &View) -> Card {
        let caps = v.cfg.caps();
        let alias = v.cfg.alias.clone();
        let hub = self.hub.clone();

        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("lm-card");

        // header row: dot, name + state, chevron, switch
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        head.add_css_class("lm-card-head");
        let dot = Dot::new(30);
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
        texts.set_hexpand(true);
        texts.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(Some(&v.cfg.name));
        title.add_css_class("lm-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let sub = gtk::Label::new(None);
        sub.add_css_class("lm-sub");
        sub.set_xalign(0.0);
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        texts.append(&title);
        texts.append(&sub);
        let chevron = gtk::Image::from_icon_name("pan-down-symbolic");
        chevron.add_css_class("lm-chevron");
        let switch = gtk::Switch::new();
        switch.set_valign(gtk::Align::Center);
        head.append(&dot.area);
        head.append(&texts);
        head.append(&chevron);
        head.append(&switch);
        root.append(&head);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let bright = SliderRow::new("display-brightness-symbolic", 1.0, 100.0, 1.0, |v| format!("{}%", v.round()));
        bright.root.add_css_class("lm-big");
        body.append(&bright.root);

        // details: tabs and pages
        let details_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        details_box.set_margin_top(4);
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(120);
        stack.set_vhomogeneous(false);
        stack.set_interpolate_size(true);

        let mut pages: Vec<(&str, &str)> = Vec::new();
        if caps.color {
            pages.push(("colour", "Colour"));
        }
        pages.push(("white", "White"));
        if caps.scenes {
            pages.push(("scenes", "Scenes"));
        }

        // colour page
        let (mut wheel, mut hex, mut swatches) = (None, None, Vec::new());
        if caps.color {
            let page = gtk::Box::new(gtk::Orientation::Horizontal, 14);
            let w = Wheel::new(150);
            {
                let (hub, alias) = (hub.clone(), alias.clone());
                w.connect_picked(move |(h, s)| hub.act(&alias, Action::Hsv(h.round() as u16 % 360, (s * 100.0).round() as u8), FADE_DRAG));
            }
            page.append(&w.area);
            let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
            right.set_valign(gtk::Align::Center);
            let grid = gtk::Grid::new();
            grid.set_row_spacing(7);
            grid.set_column_spacing(7);
            for (i, c) in lights::COLORS.iter().enumerate() {
                let b = gtk::Button::new();
                b.add_css_class("lm-swatch");
                b.add_css_class(&format!("lm-sw-{:06x}", c.rgb));
                b.set_tooltip_text(Some(c.name));
                let (hub, alias, rgb) = (hub.clone(), alias.clone(), c.rgb);
                b.connect_clicked(move |_| hub.act(&alias, Action::Rgb(rgb), FADE));
                grid.attach(&b, (i % 4) as i32, (i / 4) as i32, 1, 1);
                swatches.push((c.rgb, b));
            }
            right.append(&grid);
            let entry = gtk::Entry::new();
            entry.set_placeholder_text(Some("#RRGGBB"));
            entry.set_max_length(7);
            entry.set_width_chars(8);
            entry.add_css_class("lm-hex");
            let editing = Rc::new(Cell::new(false));
            {
                let fc = gtk::EventControllerFocus::new();
                let (e1, e2) = (editing.clone(), editing.clone());
                fc.connect_enter(move |_| e1.set(true));
                fc.connect_leave(move |_| e2.set(false));
                entry.add_controller(fc);
            }
            {
                let (hub, alias, p) = (hub.clone(), alias.clone(), self.clone());
                entry.connect_activate(move |e| match parse_hex(&e.text()) {
                    Some(rgb) => hub.act(&alias, Action::Rgb(rgb), FADE),
                    None => p.toast("A colour is # and six hex digits, like #ff8800", true),
                });
            }
            right.append(&entry);
            page.append(&right);
            stack.add_named(&page, Some("colour"));
            wheel = Some(w);
            hex = Some((entry, editing));
        }

        // white page: temperature and presets
        let ct = SliderRow::new("night-light-symbolic", caps.ct_min as f64, caps.ct_max as f64, 50.0, |v| format!("{} K", v.round()));
        ct.root.add_css_class("lm-ct");
        {
            let (hub, alias) = (hub.clone(), alias.clone());
            ct.connect_changed(move |k| hub.act(&alias, Action::Ct(k.round() as u16), FADE_DRAG));
        }
        let white = gtk::Box::new(gtk::Orientation::Vertical, 10);
        white.append(&ct.root);
        let presets = gtk::FlowBox::new();
        presets.set_selection_mode(gtk::SelectionMode::None);
        presets.set_max_children_per_line(5);
        presets.set_min_children_per_line(3);
        presets.set_column_spacing(6);
        presets.set_row_spacing(6);
        presets.set_homogeneous(true);
        for w in lights::WHITES {
            let b = gtk::Button::new();
            b.add_css_class("lm-preset");
            let bx = gtk::Box::new(gtk::Orientation::Vertical, 3);
            bx.append(&gtk::Image::from_icon_name(w.icon));
            let l = gtk::Label::new(Some(w.name));
            l.add_css_class("lm-preset-label");
            bx.append(&l);
            b.set_child(Some(&bx));
            b.set_tooltip_text(Some(&format!("{} %, {} K", w.bright, w.ct.max(caps.ct_min))));
            let (hub, alias, br, k) = (hub.clone(), alias.clone(), w.bright, w.ct);
            b.connect_clicked(move |_| hub.act(&alias, Action::White(br, k), FADE));
            presets.insert(&b, -1);
        }
        white.append(&presets);
        stack.add_named(&white, Some("white"));

        // scenes page
        let (mut scenes, mut stop) = (Vec::new(), None);
        if caps.scenes {
            let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let grid = gtk::FlowBox::new();
            grid.set_selection_mode(gtk::SelectionMode::None);
            grid.set_max_children_per_line(5);
            grid.set_min_children_per_line(5);
            grid.set_column_spacing(6);
            grid.set_row_spacing(6);
            grid.set_homogeneous(true);
            for s in lights::SCENES {
                let b = gtk::Button::new();
                b.add_css_class("lm-scene");
                let bx = gtk::Box::new(gtk::Orientation::Vertical, 4);
                let sw = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                sw.add_css_class("lm-scene-swatch");
                sw.add_css_class(&format!("lm-sc-{}", s.name.to_lowercase()));
                bx.append(&sw);
                let l = gtk::Label::new(Some(s.name));
                l.add_css_class("lm-preset-label");
                bx.append(&l);
                b.set_child(Some(&bx));
                let (hub, alias, name) = (hub.clone(), alias.clone(), s.name.to_string());
                b.connect_clicked(move |_| hub.act(&alias, Action::Scene(name.clone()), FADE));
                grid.insert(&b, -1);
                scenes.push((s.name.to_string(), b));
            }
            page.append(&grid);
            let st = gtk::Button::with_label("Stop the scene");
            st.add_css_class("lm-pill");
            st.set_halign(gtk::Align::Start);
            {
                let (hub, alias) = (hub.clone(), alias.clone());
                st.connect_clicked(move |_| hub.act(&alias, Action::StopScene, FADE));
            }
            page.append(&st);
            stack.add_named(&page, Some("scenes"));
            stop = Some(st);
        }

        // tabs, only when there is more than one page
        let tab_guard = Rc::new(Cell::new(false));
        let mut tabs: Vec<(String, gtk::ToggleButton)> = Vec::new();
        if pages.len() > 1 {
            let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            bar.add_css_class("lm-tabs");
            bar.set_homogeneous(true);
            let mut first: Option<gtk::ToggleButton> = None;
            for (id, label) in &pages {
                let t = gtk::ToggleButton::with_label(label);
                t.add_css_class("lm-tab");
                if let Some(f) = &first {
                    t.set_group(Some(f));
                } else {
                    first = Some(t.clone());
                }
                let (stack, id2, guard) = (stack.clone(), id.to_string(), tab_guard.clone());
                t.connect_toggled(move |t| {
                    if t.is_active() && !guard.get() {
                        stack.set_visible_child_name(&id2);
                    }
                });
                bar.append(&t);
                tabs.push((id.to_string(), t));
            }
            details_box.append(&bar);
        }
        details_box.append(&stack);

        let details = gtk::Revealer::new();
        details.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        details.set_transition_duration(160);
        details.set_child(Some(&details_box));
        body.append(&details);

        let retry = gtk::Button::with_label("Try again");
        retry.add_css_class("lm-pill");
        retry.set_halign(gtk::Align::Start);
        retry.set_visible(false);
        {
            let (hub, p) = (hub.clone(), self.clone());
            retry.connect_clicked(move |_| {
                p.toast("Looking for the light…", false);
                let hub = hub.clone();
                std::thread::spawn(move || {
                    hub.search(true);
                    hub.refresh();
                });
            });
        }
        root.append(&body);
        root.append(&retry);

        // behaviour
        let switch_guard = Rc::new(Cell::new(false));
        {
            let (hub, alias, guard) = (hub.clone(), alias.clone(), switch_guard.clone());
            switch.connect_active_notify(move |s| {
                if !guard.get() {
                    hub.act(&alias, Action::Power(s.is_active()), FADE);
                }
            });
        }
        {
            let (hub, alias) = (hub.clone(), alias.clone());
            bright.connect_changed(move |b| hub.act(&alias, Action::Bright(b.round() as u8), FADE_DRAG));
        }
        {
            let (p, alias) = (self.clone(), alias.clone());
            let click = gtk::GestureClick::new();
            click.connect_released(move |g, _, x, _| {
                // the switch handles its own clicks
                if let Some(w) = g.widget() {
                    if let Some(hit) = w.pick(x, 20.0, gtk::PickFlags::DEFAULT) {
                        if hit.ancestor(gtk::Switch::static_type()).is_some() {
                            return;
                        }
                    }
                }
                p.toggle_open(&alias);
            });
            head.add_controller(click);
        }

        Card { alias, root, dot, title, sub, switch, switch_guard, bright, chevron, details, stack, tabs, tab_guard, ct, wheel, hex, swatches, scenes, stop, retry, body }
    }

    fn toggle_open(self: &Rc<Self>, alias: &str) {
        let now = self.open.borrow().clone();
        *self.open.borrow_mut() = if now.as_deref() == Some(alias) { None } else { Some(alias.to_string()) };
        self.sync(&self.hub.views());
    }

    /// Choose each card's tab from what its light is doing (colour, white or a scene).
    fn tabs_from_state(&self, views: &[View]) {
        for c in self.cards.borrow().iter() {
            let Some(v) = views.iter().find(|v| v.cfg.alias == c.alias) else { continue };
            let want = if v.state.flowing && c.scenes.len() > 0 {
                "scenes"
            } else if matches!(v.state.mode, Mode::Color | Mode::Hsv) && c.wheel.is_some() {
                "colour"
            } else {
                "white"
            };
            c.tab_guard.set(true);
            for (id, t) in &c.tabs {
                t.set_active(id == want);
            }
            c.tab_guard.set(false);
            c.stack.set_visible_child_name(want);
        }
    }

    pub fn sync(&self, views: &[View]) {
        let open = self.open.borrow().clone();
        for c in self.cards.borrow().iter() {
            let Some(v) = views.iter().find(|v| v.cfg.alias == c.alias) else { continue };
            let s = &v.state;
            let caps: Caps = v.cfg.caps();
            let offline = s.online == Some(false);
            c.title.set_text(&v.cfg.name);
            c.dot.set(s.shown_rgb(), s.power && !offline);
            c.sub.set_text(&describe(v));
            c.sub.set_tooltip_text(s.error.as_deref());
            c.root.set_css_classes(&if offline { vec!["lm-card", "offline"] } else { vec!["lm-card"] });
            c.switch_guard.set(true);
            c.switch.set_active(s.power && !offline);
            c.switch_guard.set(false);
            c.switch.set_sensitive(!offline);
            c.body.set_sensitive(!offline && s.online.is_some());
            c.retry.set_visible(offline);
            c.bright.set_quiet(s.bright as f64);
            c.ct.set_quiet(s.ct.clamp(caps.ct_min, caps.ct_max) as f64);
            if let Some(w) = &c.wheel {
                match s.mode {
                    Mode::Hsv => w.set_quiet(s.hue as f64, s.sat as f64 / 100.0),
                    Mode::Color => {
                        let (r, g, b) = lights::split(s.rgb);
                        let (h, sat, _) = lights::rgb_to_hsv(r, g, b);
                        w.set_quiet(h, sat);
                    }
                    Mode::White => {}
                }
            }
            if let Some((e, editing)) = &c.hex {
                if !editing.get() {
                    let (r, g, b) = s.shown_rgb();
                    e.set_text(&if s.mode == Mode::White { String::new() } else { format!("#{:06x}", lights::join(r, g, b)) });
                }
            }
            for (rgb, b) in &c.swatches {
                set_class(b, "active", s.mode == Mode::Color && s.rgb == *rgb && !s.flowing);
            }
            for (name, b) in &c.scenes {
                set_class(b, "active", s.flowing && s.scene.as_deref() == Some(name.as_str()));
            }
            if let Some(st) = &c.stop {
                st.set_visible(s.flowing);
            }
            let is_open = open.as_deref() == Some(c.alias.as_str());
            c.details.set_reveal_child(is_open);
            c.chevron.set_icon_name(Some(if is_open { "pan-up-symbolic" } else { "pan-down-symbolic" }));
        }
    }

    pub fn toast(&self, msg: &str, error: bool) {
        self.toast.set_text(msg);
        set_class(&self.toast, "error", error);
        self.toast.set_visible(true);
        let g = self.toast_gen.get() + 1;
        self.toast_gen.set(g);
        let (t, gen) = (self.toast.clone(), self.toast_gen.clone());
        glib::timeout_add_local_once(Duration::from_millis(if error { 6000 } else { 3500 }), move || {
            if gen.get() == g {
                t.set_visible(false);
            }
        });
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn show(self: &Rc<Self>) {
        self.autostart.set_active(crate::autostart::is_enabled());
        self.hub.refresh();
        let views = self.hub.views();
        self.tabs_from_state(&views);
        self.sync(&views);
        self.scrim.present();
        self.window.present();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
        self.scrim.set_visible(false);
    }

    /// After the lights answered the first time: pick tabs to match them.
    pub fn first_answers(&self) {
        self.tabs_from_state(&self.hub.views());
    }
}

fn set_class(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}

fn popover_open(window: &gtk::Window) -> bool {
    fn walk(w: &gtk::Widget) -> bool {
        if let Some(p) = w.downcast_ref::<gtk::Popover>() {
            if p.is_visible() {
                return true;
            }
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            if walk(&x) {
                return true;
            }
            c = x.next_sibling();
        }
        false
    }
    window.child().map(|c| walk(&c)).unwrap_or(false)
}

pub fn parse_hex(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('#');
    if t.len() == 6 {
        u32::from_str_radix(t, 16).ok()
    } else if t.len() == 3 {
        let v = u32::from_str_radix(t, 16).ok()?;
        let (r, g, b) = ((v >> 8) & 15, (v >> 4) & 15, v & 15);
        Some((r * 17) << 16 | (g * 17) << 8 | b * 17)
    } else {
        None
    }
}

/// The line under a light's name.
pub fn describe(v: &View) -> String {
    let s = &v.state;
    match s.online {
        None => return "Connecting…".into(),
        Some(false) => return format!("Not reachable at {}", v.cfg.ip),
        _ => {}
    }
    if !s.power {
        return "Off".into();
    }
    let what = if let (true, Some(n)) = (s.flowing, &s.scene) {
        n.clone()
    } else if s.flowing {
        "Scene".into()
    } else {
        match s.mode {
            Mode::White => lights::WHITES.iter().find(|w| w.ct == s.ct && w.bright == s.bright).map(|w| w.name.to_string()).unwrap_or_else(|| format!("{} K", s.ct)),
            Mode::Color => lights::COLORS.iter().find(|c| c.rgb == s.rgb).map(|c| c.name.to_string()).unwrap_or_else(|| format!("#{:06x}", s.rgb)),
            Mode::Hsv => {
                let (r, g, b) = s.shown_rgb();
                format!("#{:06x}", lights::join(r, g, b))
            }
        }
    };
    format!("On · {}% · {}", s.bright, what)
}

/// CSS for the colour swatches and scene strips (generated from the preset tables).
pub fn swatch_css() -> String {
    let mut css = String::new();
    for c in lights::COLORS {
        css.push_str(&format!(".lm-sw-{:06x} {{ background: #{:06x}; }}\n", c.rgb, c.rgb));
    }
    for s in lights::SCENES {
        css.push_str(&format!(
            ".lm-sc-{} {{ background-image: linear-gradient(to right, #{:06x}, #{:06x}, #{:06x}); }}\n",
            s.name.to_lowercase(),
            s.swatch[0],
            s.swatch[1],
            s.swatch[2]
        ));
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex() {
        assert_eq!(parse_hex("#ff8800"), Some(0xff8800));
        assert_eq!(parse_hex("f80"), Some(0xff8800));
        assert_eq!(parse_hex("#ff88"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
    }
}
