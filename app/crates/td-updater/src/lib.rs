//! Native 2.x update groundwork. Blocking: call from a worker, never the UI thread.
//! The caller loads AppSettings before every poll. No task, installer or 1.x bridge
//! is started here. Keep UpdateReady alive while using its temporary files, and
//! reverify immediately before any future installer launch.

mod signature;

use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use semver::Version;
use serde::Deserialize;
use td_core::settings::AppSettings;

pub const PUBLIC_KEY: &str = include_str!("../updater.pub");
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RELEASES: &str = "https://api.github.com/repos/Duresa7/OpenTouchDeck/releases";
const DOWNLOADS: &str = "https://github.com/Duresa7/OpenTouchDeck/releases/download";
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 10;
const MAX_FEED: u64 = 2 * 1024 * 1024;
const MAX_SIGNATURE: u64 = 16 * 1024;
const MAX_MSI: u64 = 256 * 1024 * 1024;
/// Every staging folder is `<temp>/touchdeck-update-<random>`, so a later run can find
/// folders that a killed process left behind.
const STAGING_PREFIX: &str = "touchdeck-update-";
/// Longest wait for any single read. TCP retransmits a lost segment within about 7 s
/// (1 + 2 + 4 s), so a healthy slow link still makes progress, while a stalled body
/// read fails in seconds rather than holding the worker for the 5-minute global limit.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum Failure {
    #[error("update check cancelled")]
    Cancelled,
    #[error("update request failed: {0}")]
    Http(String),
    #[error("invalid release feed: {0}")]
    Feed(String),
    #[error("release {available} is older than installed version {current}")]
    LowerVersion {
        current: Version,
        available: Version,
    },
    #[error("invalid or missing updater signature")]
    Signature,
    #[error("update file I/O failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug)]
pub enum UpdateResult {
    UpToDate,
    UpdateReady(UpdateReady),
    Failed(Failure),
}

/// Owns the private temporary directory; dropping it deletes both downloads.
#[derive(Debug)]
pub struct UpdateReady {
    directory: tempfile::TempDir,
    path: PathBuf,
    signature_path: PathBuf,
    version: Version,
}

impl UpdateReady {
    /// Recheck the kept key immediately before handing the MSI to Windows Installer.
    pub fn verify(&self) -> Result<(), Failure> {
        let wrapped = std::fs::read(&self.signature_path)?;
        let filename = format!("OpenTouchDeck-{}-windows-x64.msi", self.version);
        signature::verify(&self.path, &wrapped, PUBLIC_KEY, &filename, &self.version)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn signature_path(&self) -> &Path {
        &self.signature_path
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn directory(&self) -> &Path {
        self.directory.path()
    }
}

/// Transport must reject non-success status codes and non-HTTPS redirects.
/// Returned readers must enforce a request timeout. No credentials are needed.
pub trait HttpClient {
    fn get(&self, url: &str, user_agent: &str) -> Result<Box<dyn Read>, Failure>;
}

pub struct GitHubClient(ureq::Agent);

impl Default for GitHubClient {
    fn default() -> Self {
        Self(
            ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(5)
                .timeout_global(Some(Duration::from_secs(5 * 60)))
                .timeout_per_read(Some(READ_TIMEOUT))
                .build()
                .into(),
        )
    }
}

impl HttpClient for GitHubClient {
    fn get(&self, url: &str, user_agent: &str) -> Result<Box<dyn Read>, Failure> {
        let response = self
            .0
            .get(url)
            .header("User-Agent", user_agent)
            .header(
                "Accept",
                if url.starts_with(RELEASES) {
                    "application/vnd.github+json"
                } else {
                    "application/octet-stream"
                },
            )
            .call()
            .map_err(|error| Failure::Http(error.to_string()))?;
        if !response.status().is_success() {
            return Err(Failure::Http(format!("HTTP {}", response.status())));
        }
        Ok(Box::new(response.into_body().into_reader()))
    }
}

/// Monotonic elapsed time, so changing the PC's date doesn't trigger extra checks.
pub trait Clock {
    fn now(&self) -> Duration;
}

pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

pub struct Updater<H, C> {
    http: H,
    clock: C,
    current: Version,
    last_check: Option<Duration>,
    public_key: &'static str,
    staging_root: PathBuf,
}

impl<H: HttpClient, C: Clock> Updater<H, C> {
    /// The installed app supplies its version; the trusted key cannot be overridden.
    pub fn new(http: H, clock: C, current: Version) -> Self {
        Self {
            http,
            clock,
            current,
            last_check: None,
            public_key: PUBLIC_KEY,
            staging_root: std::env::temp_dir(),
        }
    }

    /// Removes staging folders that an earlier run left behind, for example when the
    /// process exited while a download was still blocked in a read. Call it once at
    /// startup, before the first poll, while holding the single-instance lock: it
    /// assumes no other staging folder of this app is in use. Only folders named with
    /// this app's prefix that hold nothing but this app's MSI and signature files are
    /// touched. Returns how many were removed; a folder that cannot be removed is skipped.
    pub fn remove_stale_staging(&self) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.staging_root) else {
            return 0;
        };
        entries
            .flatten()
            .filter(|entry| is_stale_staging(&entry.path()))
            .filter(|entry| std::fs::remove_dir_all(entry.path()).is_ok())
            .count()
    }

    /// First enabled poll checks immediately; later attempts are 24 hours apart,
    /// including failures. None means disabled, not due, or cancelled.
    /// Cancellation is cooperative: an in-flight request/read must return first.
    /// This is a polling API, not a background scheduler.
    pub fn poll(
        &mut self,
        settings: &AppSettings,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<UpdateResult> {
        if cancelled() || !settings.automatic_update_checks {
            return None;
        }
        let now = self.clock.now();
        if self
            .last_check
            .is_some_and(|last| now.saturating_sub(last) < CHECK_INTERVAL)
        {
            return None;
        }
        self.last_check = Some(now);
        let result = self.check(cancelled);
        if cancelled() {
            return None;
        }
        Some(match result {
            Ok(Some(ready)) => UpdateResult::UpdateReady(ready),
            Ok(None) => UpdateResult::UpToDate,
            Err(Failure::Cancelled) => return None,
            Err(error) => UpdateResult::Failed(error),
        })
    }

    fn small(
        &self,
        url: &str,
        limit: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, Failure> {
        check_cancelled(cancelled)?;
        let response = self
            .http
            .get(url, &format!("OpenTouchDeck/{}", self.current))?;
        check_cancelled(cancelled)?;
        let mut bytes = Vec::new();
        response.take(limit + 1).read_to_end(&mut bytes)?;
        check_cancelled(cancelled)?;
        if bytes.len() as u64 > limit {
            return Err(Failure::Feed("response too large".into()));
        }
        Ok(bytes)
    }

    fn newest(&self, cancelled: &dyn Fn() -> bool) -> Result<Option<(Version, Release)>, Failure> {
        let mut newest: Option<(Version, Release)> = None;
        for page in 1..=MAX_PAGES {
            let bytes = self.small(
                &format!("{RELEASES}?per_page={PAGE_SIZE}&page={page}"),
                MAX_FEED,
                cancelled,
            )?;
            let releases: Vec<Release> =
                serde_json::from_slice(&bytes).map_err(|error| Failure::Feed(error.to_string()))?;
            let done = releases.len() < PAGE_SIZE;
            for release in releases {
                let Ok(version) = Version::parse(
                    release
                        .tag_name
                        .strip_prefix('v')
                        .unwrap_or(&release.tag_name),
                ) else {
                    continue;
                };
                if release.draft
                    || release.prerelease
                    || version.major != 2
                    || !version.pre.is_empty()
                {
                    continue;
                }
                if newest
                    .as_ref()
                    .is_none_or(|(best, _)| version.cmp_precedence(best).is_gt())
                {
                    newest = Some((version, release));
                }
            }
            if done {
                return Ok(newest);
            }
        }
        Err(Failure::Feed("release pagination limit exceeded".into()))
    }

    fn check(&self, cancelled: &dyn Fn() -> bool) -> Result<Option<UpdateReady>, Failure> {
        let Some((version, release)) = self.newest(cancelled)? else {
            return Ok(None);
        };
        match version.cmp_precedence(&self.current) {
            std::cmp::Ordering::Less => {
                return Err(Failure::LowerVersion {
                    current: self.current.clone(),
                    available: version,
                });
            }
            std::cmp::Ordering::Equal => return Ok(None),
            std::cmp::Ordering::Greater => {}
        }
        let filename = format!("OpenTouchDeck-{version}-windows-x64.msi");
        let msi = release.asset(&filename)?;
        let signature = release.asset(&format!("{filename}.sig"))?;
        if msi.size == 0
            || msi.size > MAX_MSI
            || signature.size == 0
            || signature.size > MAX_SIGNATURE
        {
            return Err(Failure::Feed("invalid asset size".into()));
        }
        // Require exactly this repository, tag and filename, not arbitrary URLs
        // or paths taken from release metadata. Redirects remain HTTPS-only.
        for asset in [msi, signature] {
            if asset.browser_download_url
                != format!("{DOWNLOADS}/{}/{name}", release.tag_name, name = asset.name)
            {
                return Err(Failure::Feed("unexpected asset URL".into()));
            }
        }
        let wrapped = self.small(&signature.browser_download_url, MAX_SIGNATURE, cancelled)?;
        if wrapped.len() as u64 != signature.size {
            return Err(Failure::Signature);
        }
        check_cancelled(cancelled)?;
        let mut response = self
            .http
            .get(
                &msi.browser_download_url,
                &format!("OpenTouchDeck/{}", self.current),
            )?
            .take(msi.size + 1);
        check_cancelled(cancelled)?;
        // TempDir removes partial staging on cancellation or any other error.
        let directory = tempfile::Builder::new()
            .prefix(STAGING_PREFIX)
            .tempdir_in(&self.staging_root)?;
        let path = directory.path().join(&filename);
        let signature_path = directory.path().join(format!("{filename}.sig"));
        check_cancelled(cancelled)?;
        std::fs::write(&signature_path, &wrapped)?;
        check_cancelled(cancelled)?;
        let mut file = File::options().write(true).create_new(true).open(&path)?;
        let mut copied = 0;
        let mut buffer = [0; 64 * 1024];
        loop {
            check_cancelled(cancelled)?;
            let count = match response.read(&mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            // A read can block while shutdown is requested. Never write its bytes then.
            check_cancelled(cancelled)?;
            if count == 0 {
                break;
            }
            file.write_all(&buffer[..count])?;
            copied += count as u64;
        }
        if copied != msi.size {
            return Err(Failure::Signature);
        }
        check_cancelled(cancelled)?;
        file.sync_all()?;
        drop(file);
        signature::verify(&path, &wrapped, self.public_key, &filename, &version)?;
        check_cancelled(cancelled)?;
        Ok(Some(UpdateReady {
            directory,
            path,
            signature_path,
            version,
        }))
    }
}

/// A real directory (not a link) named like this app's staging folders, containing only
/// regular files named like this app's installer or its signature.
fn is_stale_staging(path: &Path) -> bool {
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(STAGING_PREFIX))
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_alphanumeric())
        });
    if !named
        || !std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
    {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.into_iter().all(|entry| {
        entry.is_ok_and(|entry| {
            let ours = entry.file_name().to_str().is_some_and(|name| {
                name.strip_prefix("OpenTouchDeck-").is_some_and(|rest| {
                    rest.ends_with("-windows-x64.msi") || rest.ends_with("-windows-x64.msi.sig")
                })
            });
            ours && entry.file_type().is_ok_and(|kind| kind.is_file())
        })
    })
}

fn check_cancelled(cancelled: &dyn Fn() -> bool) -> Result<(), Failure> {
    if cancelled() {
        Err(Failure::Cancelled)
    } else {
        Ok(())
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

impl Release {
    fn asset(&self, name: &str) -> Result<&Asset, Failure> {
        let mut matches = self.assets.iter().filter(|asset| asset.name == name);
        let asset = matches.next().ok_or(Failure::Signature)?;
        if matches.next().is_some() {
            return Err(Failure::Feed("duplicate asset".into()));
        }
        Ok(asset)
    }
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[cfg(test)]
mod tests;
