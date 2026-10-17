//! Start with the session, and a launcher entry, through plain XDG files.

use std::path::PathBuf;

fn exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "lumen".into())
}

fn entry() -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Lumen\nGenericName=Lights\nComment=Your Xiaomi lights over the local network\nExec={} run\nIcon=lumen\nTerminal=false\nCategories=Utility;\nKeywords=light;lamp;bulb;yeelight;xiaomi;\nStartupNotify=false\n",
        exe()
    )
}

fn autostart_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.config_dir().join("autostart").join("lumen.desktop"))
}

pub fn is_enabled() -> bool {
    autostart_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn set(on: bool) -> Result<PathBuf, String> {
    let p = autostart_path().ok_or("no config directory")?;
    if on {
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, entry()).map_err(|e| e.to_string())?;
    } else {
        let _ = std::fs::remove_file(&p);
    }
    Ok(p)
}

pub fn install_launcher() -> Result<PathBuf, String> {
    let d = directories::BaseDirs::new().ok_or("no data directory")?;
    let p = d.data_dir().join("applications").join("lumen.desktop");
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&p, entry()).map_err(|e| e.to_string())?;
    let dir = d.data_dir().join("icons/hicolor/scalable/apps");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("lumen.svg"), include_str!("../data/lumen.svg")).map_err(|e| e.to_string())?;
    Ok(p)
}
