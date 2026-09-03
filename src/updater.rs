use anyhow::{Context, Result, bail};
use cargo_packager_updater::{Config, UpdaterBuilder, semver::Version, url::Url};
use std::{sync::mpsc, thread, time::Duration};

const UPDATE_ENDPOINT: &str =
    "https://github.com/vdeng-ai/LLM2MCP/releases/latest/download/latest.json";

#[derive(Debug, Clone)]
pub enum UpdateEvent {
    UpToDate,
    Available {
        version: String,
        notes: Option<String>,
    },
    Installed {
        version: String,
    },
    Error(String),
}

pub fn enabled() -> bool {
    embedded_public_key().is_some()
}

pub fn automatic_install_supported() -> bool {
    if !enabled() {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        // cargo-packager-updater can replace an AppImage in place, but it does
        // not install Debian packages. A .deb launch normally resolves to
        // /usr/bin/llm2mcp and must be upgraded through the package manager.
        std::env::var_os("APPIMAGE").is_some()
    }

    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

pub fn spawn_check() -> Option<mpsc::Receiver<UpdateEvent>> {
    if !enabled() {
        return None;
    }
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let event = match check() {
            Ok(Some((version, notes))) => UpdateEvent::Available { version, notes },
            Ok(None) => UpdateEvent::UpToDate,
            Err(error) => UpdateEvent::Error(format!("{error:#}")),
        };
        let _ = sender.send(event);
    });
    Some(receiver)
}

pub fn spawn_install() -> Option<mpsc::Receiver<UpdateEvent>> {
    if !automatic_install_supported() {
        return None;
    }
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let event = match install_latest() {
            Ok(Some(version)) => UpdateEvent::Installed { version },
            Ok(None) => UpdateEvent::UpToDate,
            Err(error) => UpdateEvent::Error(format!("{error:#}")),
        };
        let _ = sender.send(event);
    });
    Some(receiver)
}

fn embedded_public_key() -> Option<&'static str> {
    option_env!("LLM2MCP_UPDATER_PUBKEY")
        .map(str::trim)
        .filter(|key| !key.is_empty())
}

fn build_updater() -> Result<cargo_packager_updater::Updater> {
    let pubkey = embedded_public_key()
        .context("automatic updates are disabled because this build has no updater public key")?;
    let endpoint = Url::parse(UPDATE_ENDPOINT).context("invalid built-in updater endpoint")?;
    let current = Version::parse(env!("CARGO_PKG_VERSION")).context("invalid package version")?;
    let config = Config {
        endpoints: vec![endpoint],
        pubkey: pubkey.to_owned(),
        ..Default::default()
    };
    UpdaterBuilder::new(current, config)
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to initialize updater")
}

fn check() -> Result<Option<(String, Option<String>)>> {
    let updater = build_updater()?;
    let update = updater.check().context("failed to check for updates")?;
    Ok(update.map(|update| (update.version, update.body)))
}

fn install_latest() -> Result<Option<String>> {
    let updater = build_updater()?;
    let Some(update) = updater.check().context("failed to check for updates")? else {
        return Ok(None);
    };
    let version = update.version.clone();
    update
        .download_and_install()
        .with_context(|| format!("failed to install LLM2MCP {version}"))?;
    if version.trim().is_empty() {
        bail!("updater installed a release without a version");
    }
    Ok(Some(version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_endpoint_is_https_and_points_to_latest_manifest() {
        let url = Url::parse(UPDATE_ENDPOINT).expect("valid endpoint");
        assert_eq!(url.scheme(), "https");
        assert!(
            url.path()
                .ends_with("/releases/latest/download/latest.json")
        );
    }
}
