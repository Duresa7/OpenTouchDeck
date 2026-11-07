//! Provenance and the 30-day rule. Brandfetch's terms (§5.1) let content fetched through its
//! API be cached for at most 30 days from retrieval, and ask for a new API call at least every
//! 30 days, or the cached content must go. So each Brandfetch Asset records where it came from
//! and when, the original file stays cached with that time, and a check every hour calls the
//! Brand API again for each logo 25 days old:
//!
//! - same file: the fetch time moves to now;
//! - a changed file: it renders a new Asset, and the provenance records which Asset replaced
//!   the old one. The UI swaps it into its Layout drafts, so the next push updates the Device.
//!   Nothing is pushed here and no stored Layout changes;
//! - a failure that may pass, such as no answer or used-up requests: it tries again a day later;
//! - a failure that won't: Brandfetch refused the key, or no longer knows the brand or has no
//!   logo for it. It stops calling for that logo, which expires. Saving a key resumes the ones
//!   the old key failed;
//! - past 30 days with no success: the cached original is deleted and the logo is marked
//!   expired. An expired logo is never called again; picking it again starts over. A Device's
//!   Asset is never deleted here.
//!
//! Each call costs the user one of their Brand API requests, so a run makes at most
//! [`CALLS_PER_RUN`] and leaves the rest for the next run.
use super::{
    Brandfetch, BrandfetchError, BrandfetchLogo,
    client::{FileFormat, Original, draw},
};
use crate::{api::Ctx, features::assets::Fit, store::AssetStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use td_link::asset::valid_asset_id;

const DAY_S: u64 = 24 * 60 * 60;
/// A logo this old gets a new Brand API call.
pub const REFRESH_AFTER_S: u64 = 25 * DAY_S;
/// A logo this old with no successful call loses its cached original.
pub const EXPIRE_AFTER_S: u64 = 30 * DAY_S;
/// A failed refresh waits this long before the next try. The hour of slack keeps a check that
/// runs a little early from skipping a day.
const RETRY_AFTER_S: u64 = DAY_S - 60 * 60;
/// Brand API calls one refresh run makes at most.
pub const CALLS_PER_RUN: usize = 10;
/// How often the refresh looks for due logos. It compares wall-clock times, so after the PC
/// sleeps through a day it catches up within this.
pub const REFRESH_TICK: Duration = Duration::from_secs(60 * 60);

/// Why a logo's refresh stopped calling Brandfetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Halt {
    /// Brandfetch refused the key. Saving a key resumes the refresh.
    Key,
    /// Brandfetch no longer knows the brand or has no logo for it.
    Gone,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    logo: BrandfetchLogo,
    fit: Fit,
    /// The SHA-256 of the cached original, which names its file. Empty once expired.
    original: String,
    format: FileFormat,
    /// Unix seconds of the last successful Brand API call for this logo.
    fetched_at: u64,
    /// Unix seconds of the last refresh attempt, successful or not.
    #[serde(default)]
    tried_at: u64,
    #[serde(default)]
    expired: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    halted: Option<Halt>,
}
impl Record {
    fn due(&self, now: u64) -> bool {
        !self.expired
            && self.halted.is_none()
            && now.saturating_sub(self.fetched_at) >= REFRESH_AFTER_S
            && (self.tried_at <= self.fetched_at
                || now.saturating_sub(self.tried_at) >= RETRY_AFTER_S)
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    logos: BTreeMap<String, Record>,
    /// Each Asset a refresh replaced, and the Asset that replaced it.
    #[serde(default)]
    replaced_by: BTreeMap<String, String>,
}

/// What one refresh run did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RefreshReport {
    /// Logos whose file was unchanged, now fetched again.
    pub renewed: u32,
    /// `(old, new)` Asset ids for logos whose file changed.
    pub replaced: Vec<(String, String)>,
    /// Refresh attempts that failed and will be retried.
    pub failed: u32,
    /// Logos that stopped calling Brandfetch in this run.
    pub halted: u32,
    /// Due logos left for the next run, past [`CALLS_PER_RUN`].
    pub deferred: u32,
    /// Logos whose Asset is gone from this PC, so their provenance went too.
    pub dropped: u32,
    /// Logos that passed 30 days without a successful refresh in this run.
    pub expired: u32,
}

/// What the Settings section counts.
pub(super) struct Summary {
    pub logos: u32,
    pub need_refresh: u32,
    pub expired: Vec<String>,
    pub replaced_by: BTreeMap<String, String>,
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Refreshes due logos at start and then every `tick`, by the Companion's wall clock.
pub(crate) async fn run(ctx: Ctx, tick: Duration) {
    loop {
        let now = ctx.clock.unix_s();
        let work = ctx.clone();
        let _ =
            tokio::task::spawn_blocking(move || work.brandfetch.refresh(&work.assets, now)).await;
        tokio::time::sleep(tick).await;
    }
}

impl Brandfetch {
    fn book_path(&self) -> PathBuf {
        self.0.dir.join("provenance.json")
    }
    fn originals(&self) -> PathBuf {
        self.0.dir.join("originals")
    }
    fn original_path(&self, sha: &str, format: FileFormat) -> PathBuf {
        self.originals()
            .join(format!("{sha}.{}", format.extension()))
    }
    /// A missing or unreadable file reads as no logos. Ids name files, so a record whose Asset
    /// id or original isn't a SHA-256 in lowercase hex is dropped.
    fn read_book(&self) -> Book {
        let mut book: Book = std::fs::read(self.book_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        book.logos.retain(|id, record| {
            valid_asset_id(id) && (record.original.is_empty() || valid_asset_id(&record.original))
        });
        book.replaced_by
            .retain(|old, new| valid_asset_id(old) && valid_asset_id(new));
        book
    }
    fn write_book(&self, book: &Book) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(book).map_err(std::io::Error::other)?;
        td_core::app_dirs::write_atomic(&self.book_path(), &bytes)?;
        // Originals no record names any more go too.
        let keep: std::collections::BTreeSet<_> = book
            .logos
            .values()
            .filter(|r| !r.original.is_empty())
            .map(|r| {
                self.original_path(&r.original, r.format)
                    .file_name()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        for entry in std::fs::read_dir(self.originals())
            .into_iter()
            .flatten()
            .flatten()
        {
            if !keep.contains(&entry.file_name()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Ok(())
    }
    fn save_original(&self, original: &Original) -> std::io::Result<String> {
        let sha = sha(&original.bytes);
        td_core::app_dirs::write_atomic(
            &self.original_path(&sha, original.format),
            &original.bytes,
        )?;
        Ok(sha)
    }
    /// Records a freshly picked logo: its options, its original file and the fetch time.
    pub(super) fn record(
        &self,
        asset: &str,
        logo: &BrandfetchLogo,
        fit: Fit,
        original: &Original,
        now: u64,
    ) -> std::io::Result<()> {
        let _book = self.0.book.lock();
        let sha = self.save_original(original)?;
        let mut book = self.read_book();
        book.logos.insert(
            asset.into(),
            Record {
                logo: logo.clone(),
                fit,
                original: sha,
                format: original.format,
                fetched_at: now,
                tried_at: now,
                expired: false,
                halted: None,
            },
        );
        // A pick that gives an Asset back is current again, whatever replaced it before.
        book.replaced_by.remove(asset);
        self.write_book(&book)
    }
    /// After a new key is saved, logos the old key failed for are refreshed again.
    pub(super) fn resume_after_new_key(&self) -> std::io::Result<()> {
        let _book = self.0.book.lock();
        let mut book = self.read_book();
        let mut changed = false;
        for record in book.logos.values_mut() {
            if record.halted == Some(Halt::Key) {
                record.halted = None;
                changed = true;
            }
        }
        if changed {
            self.write_book(&book)
        } else {
            Ok(())
        }
    }
    /// The counts and replacements the Settings section and the editor read, at `now`.
    pub(super) fn summary(&self, now: u64) -> Summary {
        let book = self.read_book();
        let need_refresh = book
            .logos
            .values()
            .filter(|r| r.expired || now.saturating_sub(r.fetched_at) >= REFRESH_AFTER_S)
            .count();
        let expired = book
            .logos
            .iter()
            .filter(|(_, r)| r.expired)
            .map(|(id, _)| id.clone())
            .collect();
        Summary {
            logos: book.logos.len() as u32,
            need_refresh: need_refresh as u32,
            expired,
            replaced_by: book.replaced_by,
        }
    }
    /// Forgets logos whose Asset is gone from this PC, and replacements that lead nowhere.
    fn drop_missing(&self, assets: &AssetStore) -> u32 {
        let _book = self.0.book.lock();
        let mut book = self.read_book();
        let (logos, replaced) = (book.logos.len(), book.replaced_by.len());
        book.logos.retain(|id, _| assets.has(id));
        book.replaced_by.retain(|_, new| assets.has(new));
        let dropped = logos - book.logos.len();
        if (dropped > 0 || replaced != book.replaced_by.len()) && self.write_book(&book).is_err() {
            eprintln!("Companion: couldn't forget Brandfetch logos whose Asset is gone.");
        }
        dropped as u32
    }
    /// One refresh run at `now`, in Unix seconds. Blocking: it calls the Brand API once per due
    /// logo, at most [`CALLS_PER_RUN`] times.
    pub fn refresh(&self, assets: &AssetStore, now: u64) -> RefreshReport {
        let _running = self.0.refreshing.lock();
        let mut report = RefreshReport {
            dropped: self.drop_missing(assets),
            ..RefreshReport::default()
        };
        let due: Vec<(String, BrandfetchLogo)> = self
            .read_book()
            .logos
            .into_iter()
            .filter(|(_, r)| r.due(now))
            .map(|(id, r)| (id, r.logo))
            .collect();
        let mut calls = 0;
        // Once Brandfetch refuses the key or says the requests are used up, the other logos
        // would get the same answer: they wait without a call.
        let mut stop: Option<BrandfetchError> = None;
        for (asset, logo) in due {
            let fetched = match &stop {
                Some(BrandfetchError::KeyRefused) => Err(BrandfetchError::KeyRefused),
                Some(_) => Err(BrandfetchError::Quota),
                None if calls == CALLS_PER_RUN => {
                    report.deferred += 1;
                    continue;
                }
                None => {
                    calls += 1;
                    // The network call runs outside the provenance lock, so a pick isn't held up.
                    self.fetch(&logo, &|| false)
                }
            };
            let _book = self.0.book.lock();
            let mut book = self.read_book();
            let Some(record) = book.logos.get(&asset).cloned() else {
                continue;
            };
            let original = match fetched {
                Ok(original) => original,
                Err(error) => {
                    let entry = book.logos.get_mut(&asset).unwrap();
                    entry.tried_at = now;
                    match error {
                        BrandfetchError::KeyRefused => {
                            entry.halted = Some(Halt::Key);
                            report.halted += 1;
                            stop = Some(BrandfetchError::KeyRefused);
                        }
                        BrandfetchError::NotFound | BrandfetchError::NoLogo => {
                            entry.halted = Some(Halt::Gone);
                            report.halted += 1;
                        }
                        BrandfetchError::Quota => {
                            report.failed += 1;
                            stop = Some(BrandfetchError::Quota);
                        }
                        _ => report.failed += 1,
                    }
                    let _ = self.write_book(&book);
                    continue;
                }
            };
            let sha = sha(&original.bytes);
            let new = if sha == record.original {
                None
            } else {
                match draw(&original, &record.logo, record.fit)
                    .ok()
                    .and_then(|image| assets.put(&td_host::icon_pack::pack_any(&image)).ok())
                {
                    Some(id) => Some(id),
                    None => {
                        report.failed += 1;
                        book.logos.get_mut(&asset).unwrap().tried_at = now;
                        let _ = self.write_book(&book);
                        continue;
                    }
                }
            };
            if self.save_original(&original).is_err() {
                report.failed += 1;
                book.logos.get_mut(&asset).unwrap().tried_at = now;
                let _ = self.write_book(&book);
                continue;
            }
            let renewed = Record {
                original: sha,
                format: original.format,
                fetched_at: now,
                tried_at: now,
                expired: false,
                halted: None,
                ..record
            };
            match new {
                Some(new) if new != asset => {
                    // Earlier replacements lead to the newest Asset, so a draft that still
                    // shows an older one skips straight to it.
                    for target in book.replaced_by.values_mut() {
                        if *target == asset {
                            target.clone_from(&new);
                        }
                    }
                    book.replaced_by.insert(asset.clone(), new.clone());
                    book.replaced_by.remove(&new);
                    book.logos.remove(&asset);
                    book.logos.insert(new.clone(), renewed);
                    report.replaced.push((asset, new));
                }
                _ => {
                    book.logos.insert(asset, renewed);
                    report.renewed += 1;
                }
            }
            if self.write_book(&book).is_err() {
                eprintln!("Companion: couldn't record a refreshed Brandfetch logo.");
            }
        }
        // Expiry also covers logos whose retry waits for tomorrow, and halted ones.
        let _book = self.0.book.lock();
        let mut book = self.read_book();
        let mut changed = false;
        for record in book.logos.values_mut() {
            if !record.expired && now.saturating_sub(record.fetched_at) >= EXPIRE_AFTER_S {
                record.expired = true;
                record.original.clear();
                report.expired += 1;
                changed = true;
            }
        }
        if changed && self.write_book(&book).is_err() {
            eprintln!("Companion: couldn't record expired Brandfetch logos.");
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::brandfetch::{LogoTheme, LogoType, client};
    use parking_lot::Mutex;
    use std::sync::Arc;

    /// Answers the Brand API with one SVG logo per brand, or with `status`, or not at all. The
    /// test changes `color` to change the logo of `example.com`.
    struct Fake {
        color: Mutex<String>,
        status: Mutex<u16>,
        down: Mutex<bool>,
        calls: Mutex<u32>,
    }
    impl client::Http for Arc<Fake> {
        fn get(&self, url: &str, _: Option<&client::ApiKey>) -> Result<client::Response, String> {
            if *self.down.lock() {
                return Err("offline".into());
            }
            let ok = |body: String| client::Response {
                status: 200,
                reason: None,
                body: body.into_bytes(),
            };
            if let Some(brand) = url.split("/v2/brands/").nth(1) {
                *self.calls.lock() += 1;
                let status = *self.status.lock();
                if status != 200 {
                    return Ok(client::Response {
                        status,
                        reason: None,
                        body: vec![],
                    });
                }
                return Ok(ok(format!(
                    r#"{{"logos":[{{"type":"icon","theme":"light","formats":[{{"src":"https://asset.brandfetch.io/{brand}.svg","format":"svg"}}]}}]}}"#
                )));
            }
            let brand = url.rsplit('/').next().unwrap().trim_end_matches(".svg");
            let color = if brand == "example.com" {
                self.color.lock().clone()
            } else {
                let n = brand
                    .bytes()
                    .fold(0u32, |h, b| h.wrapping_mul(31) + u32::from(b));
                format!("#{:06x}", n & 0xff_ffff)
            };
            Ok(ok(format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="{color}"/></svg>"#
            )))
        }
    }
    struct Rig {
        dir: tempfile::TempDir,
        bf: Brandfetch,
        fake: Arc<Fake>,
        assets: AssetStore,
    }
    const T0: u64 = 1_800_000_000;
    const DAY: u64 = DAY_S;
    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let bf = Brandfetch::new(dir.path().join("brandfetch"), true);
        let fake = Arc::new(Fake {
            color: Mutex::new("#ff0000".into()),
            status: Mutex::new(200),
            down: Mutex::new(false),
            calls: Mutex::new(0),
        });
        bf.0.net.write().http = Arc::new(fake.clone());
        bf.save_key("test-api-key".into()).unwrap();
        let assets = AssetStore::new(
            dir.path().join("assets"),
            crate::store::LayoutStore::new(dir.path().join("layouts")),
            crate::api::Clock::default(),
            crate::store::ASSET_STORE_BYTES,
        );
        Rig {
            dir,
            bf,
            fake,
            assets,
        }
    }
    fn logo(brand: &str) -> BrandfetchLogo {
        BrandfetchLogo {
            brand: brand.into(),
            logo_type: LogoType::Icon,
            theme: LogoTheme::Auto,
            padding: 4,
            background: None,
            tint: None,
        }
    }
    /// Picks a brand's logo at `T0` as the render Job does, and returns its Asset id.
    fn pick(r: &Rig, brand: &str) -> String {
        let original = r.bf.fetch(&logo(brand), &|| false).unwrap();
        let image = draw(&original, &logo(brand), Fit::Contain).unwrap();
        let id = r.assets.put(&td_host::icon_pack::pack_any(&image)).unwrap();
        r.bf.record(&id, &logo(brand), Fit::Contain, &original, T0)
            .unwrap();
        id
    }
    fn originals(r: &Rig) -> usize {
        std::fs::read_dir(r.bf.originals()).map_or(0, |d| d.count())
    }
    fn calls(r: &Rig) -> u32 {
        *r.fake.calls.lock()
    }
    fn counts(r: &Rig, now: u64) -> (u32, u32, Vec<String>) {
        let s = r.bf.summary(now);
        (s.logos, s.need_refresh, s.expired)
    }

    #[test]
    fn nothing_is_called_before_day_25() {
        let r = rig();
        pick(&r, "example.com");
        let before = calls(&r);
        let report = r.bf.refresh(&r.assets, T0 + 25 * DAY - 1);
        assert_eq!(report, RefreshReport::default());
        assert_eq!(calls(&r), before);
    }

    #[test]
    fn the_same_file_moves_the_fetch_time() {
        let r = rig();
        let id = pick(&r, "example.com");
        let report = r.bf.refresh(&r.assets, T0 + 25 * DAY);
        assert_eq!(report.renewed, 1);
        assert!(r.bf.summary(T0 + 25 * DAY).replaced_by.is_empty());
        // Fresh again: another 25 days pass before the next call, however often it checks.
        let before = calls(&r);
        for hour in 1..=24 {
            r.bf.refresh(&r.assets, T0 + 25 * DAY + hour * 3600);
        }
        r.bf.refresh(&r.assets, T0 + 49 * DAY);
        assert_eq!(calls(&r), before);
        assert_eq!(counts(&r, T0 + 49 * DAY), (1, 0, vec![]));
        assert_eq!(r.bf.summary(T0).logos, 1, "{id} keeps its record");
    }

    #[test]
    fn a_changed_file_renders_a_new_asset_and_records_the_replacement() {
        let r = rig();
        let first = pick(&r, "example.com");
        *r.fake.color.lock() = "#0000ff".into();
        let report = r.bf.refresh(&r.assets, T0 + 26 * DAY);
        assert_eq!(report.replaced.len(), 1);
        let (was, second) = report.replaced[0].clone();
        assert_eq!(was, first);
        assert_ne!(second, first);
        assert!(r.assets.has(&second));
        assert!(
            r.assets.has(&first),
            "the old Asset stays for drafts that show it"
        );
        assert_eq!(originals(&r), 1, "only the new original is kept");
        let summary = r.bf.summary(T0 + 26 * DAY);
        assert_eq!((summary.logos, summary.need_refresh), (1, 0));
        assert_eq!(
            summary.replaced_by,
            BTreeMap::from([(first.clone(), second.clone())])
        );

        // A second change leads both older Assets to the newest one.
        *r.fake.color.lock() = "#00ff00".into();
        let report = r.bf.refresh(&r.assets, T0 + 52 * DAY);
        let third = report.replaced[0].1.clone();
        assert_eq!(
            r.bf.summary(T0 + 52 * DAY).replaced_by,
            BTreeMap::from([(first.clone(), third.clone()), (second, third.clone())])
        );
        // Picking the first logo again makes it current: nothing replaces it any more.
        *r.fake.color.lock() = "#ff0000".into();
        assert_eq!(pick(&r, "example.com"), first);
        assert!(!r.bf.summary(T0).replaced_by.contains_key(&first));
    }

    #[test]
    fn a_failure_retries_the_next_day_then_expires_at_30_days_and_never_calls_again() {
        let r = rig();
        let id = pick(&r, "example.com");
        *r.fake.down.lock() = true;
        let report = r.bf.refresh(&r.assets, T0 + 25 * DAY);
        assert_eq!(report.failed, 1);
        assert_eq!(originals(&r), 1);
        // The hourly checks the same day don't call again.
        for hour in 1..=22 {
            assert_eq!(
                r.bf.refresh(&r.assets, T0 + 25 * DAY + hour * 3600),
                RefreshReport::default()
            );
        }
        for day in 26..30 {
            assert_eq!(r.bf.refresh(&r.assets, T0 + day * DAY).failed, 1);
        }
        assert_eq!(counts(&r, T0 + 29 * DAY), (1, 1, vec![]));
        let last = r.bf.refresh(&r.assets, T0 + 30 * DAY);
        assert_eq!((last.failed, last.expired), (1, 1));
        assert_eq!(originals(&r), 0, "the cached original is deleted");
        assert_eq!(counts(&r, T0 + 30 * DAY), (1, 1, vec![id.clone()]));
        // The Asset stays; the user decides what to do with the Tile.
        assert!(r.assets.has(&id));
        // An expired logo is never called again, even once Brandfetch answers.
        *r.fake.down.lock() = false;
        let before = calls(&r);
        for day in 31..40 {
            r.bf.refresh(&r.assets, T0 + day * DAY);
        }
        assert_eq!(calls(&r), before);
    }

    #[test]
    fn expiry_happens_even_while_a_retry_waits() {
        let r = rig();
        pick(&r, "example.com");
        *r.fake.down.lock() = true;
        r.bf.refresh(&r.assets, T0 + 30 * DAY - 60);
        let report = r.bf.refresh(&r.assets, T0 + 30 * DAY);
        assert_eq!((report.failed, report.expired), (0, 1));
        assert_eq!(originals(&r), 0);
    }

    #[test]
    fn without_a_key_a_logo_still_expires() {
        let r = rig();
        let id = pick(&r, "example.com");
        r.bf.forget_key().unwrap();
        let before = calls(&r);
        let report = r.bf.refresh(&r.assets, T0 + 30 * DAY);
        assert_eq!((report.failed, report.expired), (1, 1));
        assert_eq!(counts(&r, T0 + 30 * DAY).2, vec![id]);
        assert_eq!(calls(&r), before, "no key, no call");
    }

    #[test]
    fn a_brand_brandfetch_no_longer_knows_stops_calling_and_expires() {
        for status in [404, 400] {
            let r = rig();
            let id = pick(&r, "example.com");
            *r.fake.status.lock() = status;
            let report = r.bf.refresh(&r.assets, T0 + 25 * DAY);
            assert_eq!((report.halted, report.failed), (1, 0), "{status}");
            let before = calls(&r);
            for day in 26..30 {
                assert_eq!(
                    r.bf.refresh(&r.assets, T0 + day * DAY),
                    RefreshReport::default()
                );
            }
            assert_eq!(calls(&r), before, "{status}: no more calls");
            assert_eq!(r.bf.refresh(&r.assets, T0 + 30 * DAY).expired, 1);
            assert_eq!(counts(&r, T0 + 30 * DAY).2, vec![id]);
        }
    }

    #[test]
    fn a_brand_with_no_logo_left_stops_calling() {
        let r = rig();
        pick(&r, "example.com");
        r.bf.0.net.write().files = vec!["https://elsewhere.example".into()];
        let report = r.bf.refresh(&r.assets, T0 + 25 * DAY);
        assert_eq!(report.halted, 1);
        let before = calls(&r);
        r.bf.refresh(&r.assets, T0 + 27 * DAY);
        assert_eq!(calls(&r), before);
    }

    #[test]
    fn a_refused_key_halts_every_logo_after_one_call_until_a_key_is_saved() {
        let r = rig();
        for n in 0..3 {
            pick(&r, &format!("brand{n}.example"));
        }
        let before = calls(&r);
        *r.fake.status.lock() = 401;
        let report = r.bf.refresh(&r.assets, T0 + 25 * DAY);
        assert_eq!((report.halted, report.failed), (3, 0));
        assert_eq!(calls(&r), before + 1, "one call tells for all");
        r.bf.refresh(&r.assets, T0 + 27 * DAY);
        assert_eq!(calls(&r), before + 1);

        *r.fake.status.lock() = 200;
        r.bf.save_key("test-api-key-2".into()).unwrap();
        let report = r.bf.refresh(&r.assets, T0 + 28 * DAY);
        assert_eq!(report.renewed, 3);
        assert_eq!(calls(&r), before + 4);
    }

    #[test]
    fn used_up_requests_stop_the_run_and_wait_a_day() {
        let r = rig();
        for n in 0..3 {
            pick(&r, &format!("brand{n}.example"));
        }
        let before = calls(&r);
        *r.fake.status.lock() = 429;
        let report = r.bf.refresh(&r.assets, T0 + 25 * DAY);
        assert_eq!((report.failed, report.halted), (3, 0));
        assert_eq!(calls(&r), before + 1);
        r.bf.refresh(&r.assets, T0 + 25 * DAY + 3600);
        assert_eq!(calls(&r), before + 1, "nothing until tomorrow");
        *r.fake.status.lock() = 200;
        assert_eq!(r.bf.refresh(&r.assets, T0 + 26 * DAY).renewed, 3);
    }

    #[test]
    fn a_run_makes_at_most_ten_calls_and_leaves_the_rest_for_the_next() {
        let r = rig();
        for n in 0..12 {
            pick(&r, &format!("brand{n}.example"));
        }
        let before = calls(&r);
        let first = r.bf.refresh(&r.assets, T0 + 25 * DAY);
        assert_eq!((first.renewed, first.deferred), (10, 2));
        assert_eq!(calls(&r), before + CALLS_PER_RUN as u32);
        let next = r.bf.refresh(&r.assets, T0 + 25 * DAY + 3600);
        assert_eq!((next.renewed, next.deferred), (2, 0));
        assert_eq!(counts(&r, T0 + 25 * DAY + 3600).1, 0);
    }

    #[test]
    fn a_logo_whose_asset_is_gone_is_forgotten_without_a_call() {
        let r = rig();
        let gone = pick(&r, "brand0.example");
        let kept = pick(&r, "brand1.example");
        std::fs::remove_file(r.dir.path().join("assets").join(format!("{gone}.bin"))).unwrap();
        let before = calls(&r);
        let report = r.bf.refresh(&r.assets, T0 + 25 * DAY);
        assert_eq!((report.dropped, report.renewed), (1, 1));
        assert_eq!(calls(&r), before + 1);
        assert_eq!(r.bf.summary(T0).logos, 1);
        assert!(r.assets.has(&kept));
    }

    #[test]
    fn records_whose_ids_are_not_hashes_are_dropped_when_read() {
        let r = rig();
        let id = pick(&r, "example.com");
        let mut book: serde_json::Value =
            serde_json::from_slice(&std::fs::read(r.bf.book_path()).unwrap()).unwrap();
        let record = book["logos"][&id].clone();
        let mut bad = record.clone();
        bad["original"] = "../../evil".into();
        book["logos"]["../not-an-id"] = record.clone();
        book["logos"]["a".repeat(64)] = bad;
        let mut upper = record;
        upper["original"] = "A".repeat(64).into();
        book["logos"]["b".repeat(64)] = upper;
        let mut replaced = serde_json::Map::new();
        replaced.insert(id.clone(), "..".into());
        replaced.insert("..".into(), id.clone().into());
        book["replaced_by"] = replaced.into();
        std::fs::write(r.bf.book_path(), serde_json::to_vec(&book).unwrap()).unwrap();
        let read = r.bf.read_book();
        assert_eq!(read.logos.keys().collect::<Vec<_>>(), [&id]);
        assert!(read.replaced_by.is_empty());
    }
}
