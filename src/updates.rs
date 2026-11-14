//! Release metadata only: no document access, executable downloads or installation.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const STORE_URL: &str = "ms-windows-store://pdp/?ProductId=9MXR0WLW42XT";
pub const RELEASES_URL: &str = "https://github.com/gokcol/MyPDF/releases";
pub const CHANGELOG: &str = include_str!("../changelog.json");
const API: &str = "https://api.github.com/repos/gokcol/MyPDF/releases?per_page=100";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub [u32; 4]);
impl Version {
    pub fn parse(value: &str) -> Option<Self> {
        let parts: Vec<_> = value
            .strip_prefix('v')
            .unwrap_or(value)
            .split('.')
            .collect();
        if !(3..=4).contains(&parts.len()) {
            return None;
        }
        let mut version = [0; 4];
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            version[i] = part.parse().ok()?;
        }
        Some(Self(version))
    }
}

#[derive(Deserialize)]
pub struct Changelog {
    pub unreleased: Vec<String>,
    pub releases: Vec<Notes>,
}
#[derive(Deserialize)]
pub struct Notes {
    pub version: String,
    pub changes: Vec<String>,
}
pub fn changelog() -> Changelog {
    serde_json::from_str(CHANGELOG).expect("validated bundled changelog")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    MacArm,
    MacIntel,
    Windows,
    Store,
}
impl Channel {
    pub fn label(self) -> &'static str {
        match self {
            Self::MacArm | Self::MacIntel => "GitHub · Mac",
            Self::Windows => "GitHub · Windows",
            Self::Store => "Microsoft Store",
        }
    }
    pub fn url(self) -> &'static str {
        if self == Self::Store {
            STORE_URL
        } else {
            RELEASES_URL
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Available {
    /// The Store API reports availability, not the pending package's version.
    pub version: Option<String>,
    pub notes: String,
    pub url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub channel: Channel,
    pub installed: String,
    pub available: Option<Available>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<String>,
    body: Option<String>,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    state: String,
}

fn trusted_asset(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("github.com")
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none()
            && u.path().starts_with("/gokcol/MyPDF/releases/download/")
    })
}
pub fn valid_destination(channel: Channel, url: &str) -> bool {
    if channel == Channel::Store {
        url == STORE_URL
    } else {
        trusted_asset(url)
    }
}
fn matches_platform(name: &str, channel: Channel) -> bool {
    let name = name.to_ascii_lowercase();
    match channel {
        Channel::MacArm => {
            name.ends_with(".dmg") && !name.contains("intel") && !name.contains("x86_64")
        }
        Channel::MacIntel => {
            name.ends_with(".dmg")
                && ["intel", "x86_64", "universal"]
                    .iter()
                    .any(|s| name.contains(s))
        }
        Channel::Windows => name.ends_with("-setup.exe") && !name.contains("arm64"),
        Channel::Store => false,
    }
}
pub fn github_report(bytes: &[u8], channel: Channel, installed: &str) -> Result<Report> {
    ensure!(
        channel != Channel::Store,
        "Store availability must come from Microsoft Store"
    );
    let current = Version::parse(installed).context("Invalid installed version")?;
    let releases: Vec<Release> =
        serde_json::from_slice(bytes).context("Invalid GitHub release response")?;
    let latest = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease && r.published_at.is_some())
        .filter_map(|r| {
            let version = Version::parse(&r.tag_name)?;
            let asset = r.assets.iter().find(|a| {
                a.state == "uploaded"
                    && a.size > 0
                    && matches_platform(&a.name, channel)
                    && trusted_asset(&a.browser_download_url)
            })?;
            Some((
                version,
                Available {
                    version: Some(r.tag_name.trim_start_matches('v').to_owned()),
                    notes: r.body.unwrap_or_default().chars().take(30_000).collect(),
                    url: asset.browser_download_url.clone(),
                },
            ))
        })
        .max_by_key(|(v, _)| *v)
        .context("No published download is available for this platform yet")?;
    Ok(Report {
        channel,
        installed: installed.into(),
        available: (latest.0 > current).then_some(latest.1),
    })
}

async fn github(channel: Channel, installed: &str) -> Result<Report> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("MyPDF/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut response = client
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .context("Could not reach GitHub. Check your internet connection and try again.")?;
    if matches!(response.status().as_u16(), 403 | 429) {
        bail!("GitHub's request limit was reached. Please try again later.");
    }
    response = response
        .error_for_status()
        .context("GitHub could not provide update information")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 2 * 1024 * 1024,
            "Release response is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    github_report(&bytes, channel, installed)
}

/// Prepare on the UI thread so Store dialogs belong to the native app window.
pub type PendingCheck = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Report>> + Send>>;
#[cfg(not(target_os = "windows"))]
pub fn prepare(_window: Option<isize>) -> Result<PendingCheck> {
    let channel = if cfg!(target_arch = "aarch64") {
        Channel::MacArm
    } else {
        Channel::MacIntel
    };
    Ok(Box::pin(github(channel, env!("CARGO_PKG_VERSION"))))
}

/// Wait on a dedicated worker; no document or UI-thread work occurs here.
pub fn check_prepared(pending: PendingCheck) -> Result<Report> {
    #[cfg(target_os = "windows")]
    let _apartment = {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    RoUninitialize();
                }
            }
        }
        Apartment
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), pending)
            .await
            .context("The update check timed out. Please try again.")?
    })
}

#[cfg(target_os = "windows")]
pub fn prepare(window: Option<isize>) -> Result<PendingCheck> {
    use windows::{
        ApplicationModel::{Package, PackageSignatureKind},
        Services::Store::StoreContext,
        Win32::{
            Foundation::HWND, Storage::Packaging::Appx::GetCurrentPackageFullName,
            UI::Shell::IInitializeWithWindow,
        },
        core::Interface,
    };
    let mut len = 0;
    let identity = unsafe { GetCurrentPackageFullName(&mut len, None) };
    if identity.0 == 15700 {
        // APPMODEL_ERROR_NO_PACKAGE: standalone EXE installation.
        return Ok(Box::pin(github(
            Channel::Windows,
            env!("CARGO_PKG_VERSION"),
        )));
    }
    let package = Package::Current().context("Could not read Windows package identity")?;
    if package.SignatureKind()? != PackageSignatureKind::Store {
        bail!(
            "This is a sideloaded Windows package. Install the Microsoft Store edition to receive Store updates."
        );
    }
    let v = package.Id()?.Version()?;
    let installed = format!("{}.{}.{}.{}", v.Major, v.Minor, v.Build, v.Revision);
    let family = package.Id()?.FamilyName()?;
    let context = StoreContext::GetDefault()?;
    let owner = window.context("Open the MyPDF window before checking Microsoft Store updates")?;
    // Called by the desktop UI thread while eframe owns this HWND. Initialize
    // the context and start the request here; only the async wait leaves it.
    unsafe {
        context
            .cast::<IInitializeWithWindow>()?
            .Initialize(HWND(owner as *mut _))?;
    }
    let operation = context.GetAppAndOptionalStorePackageUpdatesAsync()?;
    Ok(Box::pin(async move {
        let _context = context;
        let updates = operation.await.context(
            "Microsoft Store could not check for updates. Open Microsoft Store and try again.",
        )?;
        let mut available = false;
        for update in updates {
            if update.Package()?.Id()?.FamilyName()? == family {
                available = true;
            }
        }
        Ok(Report { channel: Channel::Store, installed, available: available.then(|| Available {
            version: None, notes: "Microsoft Store has an update for MyPDF. View the new version and its release notes in the Store.".into(), url: STORE_URL.into(),
        }) })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Contacts the platform's live update service"]
    fn live_update_check() {
        let result = check_prepared(prepare(None).unwrap()).unwrap();
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    }
    fn release(v: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "tag_name":v,"draft":false,"prerelease":false,"published_at":"2026-11-13", "body":"Fixed images",
            "assets":[{"name":name,"browser_download_url":format!("https://github.com/gokcol/MyPDF/releases/download/{v}/{name}"),"state":"uploaded","size":100}]
        })
    }
    #[test]
    fn versions_compare_numerically_and_reject_prereleases() {
        assert!(Version::parse("2.10.0") > Version::parse("2.9.9"));
        assert_eq!(Version::parse("v2.4.2"), Version::parse("2.4.2.0"));
        for v in ["2.3", "2.3.1-beta", "-1.2.3", "2.3.4.5.6"] {
            assert!(Version::parse(v).is_none());
        }
    }
    #[test]
    fn only_newer_published_platform_assets_trigger_updates() {
        let mut draft = release("v9.0.0", "MyPDF.dmg");
        draft["draft"] = true.into();
        let mut pre = release("v8.0.0", "MyPDF.dmg");
        pre["prerelease"] = true.into();
        let mut malicious = release("v7.0.0", "MyPDF.dmg");
        malicious["assets"][0]["browser_download_url"] = "https://example.com/payload.dmg".into();
        let bytes = serde_json::to_vec(&vec![
            draft,
            pre,
            malicious,
            release("v3.0.0", "MyPDF-Setup.exe"),
            release("v2.4.3", "MyPDF.dmg"),
            release("v2.4.2", "MyPDF.dmg"),
        ])
        .unwrap();
        let r = github_report(&bytes, Channel::MacArm, "2.4.2").unwrap();
        assert_eq!(r.available.unwrap().version.as_deref(), Some("2.4.3"));
        assert!(
            github_report(&bytes, Channel::MacArm, "2.4.3")
                .unwrap()
                .available
                .is_none()
        );
        assert!(
            github_report(&bytes, Channel::MacArm, "2.5.0")
                .unwrap()
                .available
                .is_none()
        );
        assert!(github_report(&bytes, Channel::MacIntel, "2.4.2").is_err());
        assert!(github_report(&bytes, Channel::Store, "2.4.2").is_err());
        assert_eq!(
            github_report(&bytes, Channel::Windows, "2.4.2")
                .unwrap()
                .available
                .unwrap()
                .version
                .as_deref(),
            Some("3.0.0")
        );
    }
    #[test]
    fn bundled_notes_match_the_current_version() {
        let notes = changelog();
        assert!(
            notes
                .releases
                .iter()
                .any(|n| n.version == env!("CARGO_PKG_VERSION"))
        );
        for n in notes.releases {
            assert!(Version::parse(&n.version).is_some());
            assert!(!n.changes.is_empty());
        }
    }
}
