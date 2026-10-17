//! The tray icon (StatusNotifierItem): click for the panel, middle click to switch all lights
//! on or off, scroll for the brightness of the lights that are on.

use ksni::blocking::TrayMethods;
use ksni::menu::*;

use crate::hub::{Hub, View, FADE, FADE_DRAG};
use crate::icon;
use crate::ipc::UiMsg;
use crate::lights::{self, Action};

pub struct Tray {
    pub ui: async_channel::Sender<UiMsg>,
    pub hub: Hub,
    pub views: Vec<View>,
}

impl Tray {
    fn any_on(&self) -> bool {
        self.views.iter().any(|v| v.state.power && v.state.online != Some(false))
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "lumen".into()
    }

    fn title(&self) -> String {
        "Lumen".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let filled = self.any_on();
        [16u32, 22, 24, 32, 48, 64].iter().map(|s| ksni::Icon { width: *s as i32, height: *s as i32, data: icon::argb(*s, (235, 238, 250), filled) }).collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let lines: Vec<String> = self.views.iter().map(|v| format!("{}: {}", v.cfg.name, super::panel::describe(v))).collect();
        ksni::ToolTip { title: "Lumen".into(), description: lines.join("\n"), icon_name: String::new(), icon_pixmap: Vec::new() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.ui.try_send(UiMsg::Toggle);
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        let on = !self.any_on();
        self.hub.act_all(Action::Power(on), FADE);
    }

    fn scroll(&mut self, delta: i32, orientation: ksni::Orientation) {
        if !matches!(orientation, ksni::Orientation::Vertical) || delta == 0 {
            return;
        }
        let step: i32 = if delta < 0 { 5 } else { -5 };
        for v in self.hub.views() {
            if v.state.power && v.state.online != Some(false) {
                let b = (v.state.bright as i32 + step).clamp(1, 100) as u8;
                self.hub.act(&v.cfg.alias, Action::Bright(b), FADE_DRAG);
            }
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![StandardItem {
            label: "Show lights".into(),
            activate: Box::new(|t: &mut Self| {
                let _ = t.ui.try_send(UiMsg::Show);
            }),
            ..Default::default()
        }
        .into()];
        items.push(MenuItem::Separator);
        for v in &self.views {
            let alias = v.cfg.alias.clone();
            items.push(
                CheckmarkItem {
                    label: v.cfg.name.clone(),
                    checked: v.state.power,
                    enabled: v.state.online != Some(false),
                    activate: Box::new(move |t: &mut Self| t.hub.act(&alias, Action::Toggle, FADE)),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(MenuItem::Separator);
        items.push(StandardItem { label: "All on".into(), activate: Box::new(|t: &mut Self| t.hub.act_all(Action::Power(true), FADE)), ..Default::default() }.into());
        items.push(StandardItem { label: "All off".into(), activate: Box::new(|t: &mut Self| t.hub.act_all(Action::Power(false), FADE)), ..Default::default() }.into());
        let whites: Vec<MenuItem<Self>> = lights::WHITES
            .iter()
            .map(|w| {
                let (b, k) = (w.bright, w.ct);
                StandardItem { label: w.name.into(), activate: Box::new(move |t: &mut Self| t.hub.act_all(Action::White(b, k), FADE)), ..Default::default() }.into()
            })
            .collect();
        items.push(SubMenu { label: "All lights white".into(), submenu: whites, ..Default::default() }.into());
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.ui.try_send(UiMsg::Quit);
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

pub fn spawn(tray: Tray) -> Option<ksni::blocking::Handle<Tray>> {
    match tray.assume_sni_available(true).spawn() {
        Ok(h) => Some(h),
        Err(e) => {
            log::error!("tray icon: {e}");
            None
        }
    }
}
