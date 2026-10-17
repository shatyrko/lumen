//! D-Bus: how `lumen toggle` reaches the running app. Light commands from the command line
//! go straight to the lights; the running app sees the change at its next refresh.

use zbus::blocking::Connection;

pub const NAME: &str = "dev.alecsh.Lumen";
pub const PATH: &str = "/dev/alecsh/Lumen";
pub const IFACE: &str = "dev.alecsh.Lumen1";

#[derive(Debug, Clone)]
pub enum UiMsg {
    Toggle,
    Show,
    Hide,
    Quit,
    /// something changed the lights from outside: read them again
    Refresh,
}

struct Service {
    ui: async_channel::Sender<UiMsg>,
}

#[zbus::interface(name = "dev.alecsh.Lumen1")]
impl Service {
    fn toggle(&self) {
        let _ = self.ui.try_send(UiMsg::Toggle);
    }
    fn show(&self) {
        let _ = self.ui.try_send(UiMsg::Show);
    }
    fn hide(&self) {
        let _ = self.ui.try_send(UiMsg::Hide);
    }
    fn quit(&self) {
        let _ = self.ui.try_send(UiMsg::Quit);
    }
    fn refresh(&self) {
        let _ = self.ui.try_send(UiMsg::Refresh);
    }
}

pub fn serve(ui: async_channel::Sender<UiMsg>) -> Result<Connection, String> {
    zbus::blocking::connection::Builder::session()
        .map_err(|e| e.to_string())?
        .name(NAME)
        .map_err(|e| e.to_string())?
        .serve_at(PATH, Service { ui })
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

pub fn running() -> bool {
    let Ok(conn) = Connection::session() else { return false };
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return false };
    proxy.name_has_owner(NAME.try_into().unwrap()).unwrap_or(false)
}

pub fn send(method: &str) -> Result<(), String> {
    let conn = Connection::session().map_err(|e| e.to_string())?;
    conn.call_method(Some(NAME), PATH, Some(IFACE), method, &()).map(|_| ()).map_err(|e| e.to_string())
}
