#[cfg(target_os = "linux")]
use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use directories::BaseDirs;
#[cfg(target_os = "linux")]
use std::{fs, path::Path};

#[cfg(target_os = "linux")]
const APP_ID: &str = "llm2mcp";

#[cfg(target_os = "linux")]
pub fn register_user_desktop_entry(executable: &Path) -> Result<()> {
    let base = BaseDirs::new().context("cannot resolve home directory")?;
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(Into::into)
        .unwrap_or_else(|| base.home_dir().join(".local/share"));

    let applications_dir = data_home.join("applications");
    let icon_dir = data_home.join("icons/hicolor/scalable/apps");
    fs::create_dir_all(&applications_dir)?;
    fs::create_dir_all(&icon_dir)?;

    let icon_path = icon_dir.join(format!("{APP_ID}.svg"));
    write_if_changed(&icon_path, include_bytes!("../assets/llm2mcp-icon.svg"))?;

    let desktop = desktop_entry(executable);
    let desktop_path = applications_dir.join(format!("{APP_ID}.desktop"));
    write_if_changed(&desktop_path, desktop.as_bytes())?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn desktop_entry(executable: &Path) -> String {
    let executable = executable
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!(
        "[Desktop Entry]\nType=Application\nName=LLM2MCP\nComment=Local MCP sidecar for AI coding agents\nExec=\"{executable}\"\nIcon={APP_ID}\nTerminal=false\nCategories=Development;Utility;\nStartupNotify=true\nStartupWMClass={APP_ID}\n"
    )
}

#[cfg(target_os = "linux")]
fn write_if_changed(path: &Path, data: &[u8]) -> Result<()> {
    if fs::read(path).is_ok_and(|current| current == data) {
        return Ok(());
    }
    fs::write(path, data).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_matches_linux_window_identity() {
        let entry = desktop_entry(Path::new("/tmp/llm2mcp"));
        assert!(entry.contains("Icon=llm2mcp"));
        assert!(entry.contains("StartupWMClass=llm2mcp"));
        assert!(entry.contains("Exec=\"/tmp/llm2mcp\""));
    }
}
