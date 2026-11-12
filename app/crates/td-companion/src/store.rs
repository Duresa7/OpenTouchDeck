//! What the Companion keeps on disk under its data folder: each Device's Layouts and the Asset
//! files it prepared.
use crate::clock::Clock;
use parking_lot::Mutex;
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use td_core::{Layout, Orientation};

/// Each Device's Layouts by Device id, as `layouts/<device>/<orientation>.json` in canonical
/// form. A Layout lands here once the Companion pushes it to the Device or pulls it from it.
#[derive(Clone)]
pub struct LayoutStore {
    dir: PathBuf,
}
impl LayoutStore {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
    pub fn pushed(&self) -> Self {
        Self::new(self.dir.join("pushed-v1"))
    }
    fn path(&self, device: &str, orientation: Orientation) -> Option<PathBuf> {
        // Device ids are lowercase letters, digits and `-`, so they never leave the folder.
        let safe = !device.is_empty()
            && device
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        safe.then(|| {
            self.dir
                .join(device)
                .join(format!("{}.json", name(orientation)))
        })
    }
    /// Blocking.
    pub fn put(&self, device: &str, layout: &Layout) -> std::io::Result<()> {
        let path = self
            .path(device, layout.orientation)
            .ok_or_else(|| std::io::Error::other("Invalid Device id."))?;
        td_core::app_dirs::write_atomic(&path, &layout.to_canonical_json())
    }
    /// Blocking. A missing or unreadable file reads as no Layout.
    pub fn get(&self, device: &str, orientation: Orientation) -> Option<Layout> {
        let bytes = std::fs::read(self.path(device, orientation)?).ok()?;
        Layout::parse(&bytes).ok()
    }
    /// Blocking. Every Layout stored for the Device.
    pub fn all(&self, device: &str) -> Vec<Layout> {
        [Orientation::Landscape, Orientation::Portrait]
            .into_iter()
            .filter_map(|o| self.get(device, o))
            .collect()
    }
    /// Blocking. Every Layout stored for any Device.
    pub fn everything(&self) -> Vec<Layout> {
        std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .flat_map(|device| self.all(&device))
            .collect()
    }
}
pub fn name(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::Landscape => "landscape",
        Orientation::Portrait => "portrait",
    }
}

/// Bytes the Asset store may hold.
pub const ASSET_STORE_BYTES: u64 = 256 << 20;
/// How long an Asset stays after it was last stored or read.
pub const ASSET_RECENT: Duration = Duration::from_secs(60 * 60);

#[derive(Debug)]
pub enum PutError {
    /// Neither a tdi1 icon nor a normalized GIF.
    Invalid,
    /// Storing it would pass the quota, and every stored Asset is still in use.
    Full {
        limit: u64,
        actual: u64,
    },
    Io(std::io::Error),
}

/// Asset files: the built-in icons and the ones taken from programs, kept as
/// `assets/<id>.bin`, the name the Device stores them under. Prepared Assets count against a
/// byte quota. Past it, the least recently used Asset goes first, unless a stored Layout names
/// it, an operation pinned it, or it was stored or read within [`ASSET_RECENT`].
#[derive(Clone)]
pub struct AssetStore {
    cache: td_host::icons::IconCache,
    dir: PathBuf,
    layouts: LayoutStore,
    clock: Clock,
    quota: u64,
    index: Arc<Mutex<Option<Index>>>,
}
#[derive(Default)]
struct Index {
    files: HashMap<String, Entry>,
    pins: HashMap<String, usize>,
}
struct Entry {
    bytes: u64,
    used: Used,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Used {
    Before(SystemTime),
    At(Instant),
}

/// Keeps its Assets in the store until dropped.
pub struct AssetPin {
    index: Arc<Mutex<Option<Index>>>,
    ids: Vec<String>,
}
impl Drop for AssetPin {
    fn drop(&mut self) {
        if let Some(index) = self.index.lock().as_mut() {
            for id in &self.ids {
                if let Some(count) = index.pins.get_mut(id) {
                    *count -= 1;
                    if *count == 0 {
                        index.pins.remove(id);
                    }
                }
            }
        }
    }
}

fn sweep_stale_temps(dir: &std::path::Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if name
            .to_str()
            .and_then(|name| name.strip_prefix('.'))
            .and_then(|name| name.strip_suffix(".tmp"))
            .and_then(|name| name.split_once(".bin."))
            .is_some_and(|(id, pid)| {
                td_link::asset::valid_asset_id(id)
                    && pid
                        .parse::<u32>()
                        .is_ok_and(|value| value.to_string() == pid)
            })
            && entry.file_type().is_ok_and(|t| t.is_file())
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

impl AssetStore {
    pub(crate) fn new(dir: PathBuf, layouts: LayoutStore, clock: Clock, quota: u64) -> Self {
        sweep_stale_temps(&dir);
        Self {
            cache: td_host::icons::IconCache::new(dir.clone()),
            dir,
            layouts,
            clock,
            quota,
            index: Arc::default(),
        }
    }
    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.bin"))
    }
    /// Blocking. The file for an id, from the built-in set or the store.
    pub fn get(&self, id: &str) -> Option<Vec<u8>> {
        if td_link::asset::valid_asset_id(id) {
            let bytes = std::fs::read(self.path(id)).ok()?;
            if td_link::asset::asset_id(&bytes) != id {
                return None;
            }
            let mut index = self.index.lock();
            if let Some(entry) = self.load(&mut index).files.get_mut(id) {
                entry.used = Used::At(self.clock.now());
                self.touch(id);
            }
            return Some(bytes);
        }
        self.cache.get(id)
    }
    /// Blocking. Whether the store holds a prepared Asset with this id. Unlike [`Self::get`],
    /// it doesn't count as a use.
    pub fn has(&self, id: &str) -> bool {
        td_link::asset::valid_asset_id(id) && self.path(id).is_file()
    }
    /// Blocking. Validates and stores an Asset and returns its content hash.
    pub fn put(&self, file: &[u8]) -> Result<String, PutError> {
        if !td_host::icon_pack::is_icon(file)
            && td_host::gif_normalize::inspect_normalized(file).is_err()
        {
            return Err(PutError::Invalid);
        }
        let id = td_link::asset::asset_id(file);
        let now = self.clock.now();
        let mut guard = self.index.lock();
        let index = self.load(&mut guard);
        if let Some(entry) = index.files.get_mut(&id)
            && self.path(&id).is_file()
        {
            entry.used = Used::At(now);
            self.touch(&id);
            return Ok(id);
        }
        index.files.remove(&id);
        let size = file.len() as u64;
        let mut stored: u64 = index.files.values().map(|e| e.bytes).sum();
        if stored + size > self.quota {
            let named = self.named_by_layouts();
            let recent =
                |used: Used| matches!(used, Used::At(at) if now.duration_since(at) < ASSET_RECENT);
            let mut sweep: Vec<_> = index
                .files
                .iter()
                .filter(|(id, entry)| {
                    !named.contains(*id) && !index.pins.contains_key(*id) && !recent(entry.used)
                })
                .map(|(id, entry)| (entry.used, id.clone()))
                .collect();
            sweep.sort();
            for (_, old) in sweep {
                if stored + size <= self.quota {
                    break;
                }
                match std::fs::remove_file(self.path(&old)) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => continue,
                    _ => {}
                }
                if let Some(entry) = index.files.remove(&old) {
                    stored -= entry.bytes;
                }
            }
            if stored + size > self.quota {
                return Err(PutError::Full {
                    limit: self.quota,
                    actual: stored + size,
                });
            }
        }
        td_core::app_dirs::write_atomic(&self.path(&id), file).map_err(PutError::Io)?;
        index.files.insert(
            id.clone(),
            Entry {
                bytes: size,
                used: Used::At(now),
            },
        );
        Ok(id)
    }
    /// Keeps these Assets through any sweep until the pin is dropped.
    pub fn pin(&self, ids: impl IntoIterator<Item = String>) -> AssetPin {
        let ids: Vec<String> = ids.into_iter().collect();
        let mut guard = self.index.lock();
        let index = self.load(&mut guard);
        for id in &ids {
            *index.pins.entry(id.clone()).or_default() += 1;
        }
        AssetPin {
            index: self.index.clone(),
            ids,
        }
    }
    fn load<'a>(&self, index: &'a mut Option<Index>) -> &'a mut Index {
        index.get_or_insert_with(|| {
            let mut files = HashMap::new();
            for entry in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
                let name = entry.file_name();
                let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".bin")) else {
                    continue;
                };
                if td_link::asset::valid_asset_id(id)
                    && let Ok(meta) = entry.metadata()
                {
                    files.insert(
                        id.to_owned(),
                        Entry {
                            bytes: meta.len(),
                            used: Used::Before(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
                        },
                    );
                }
            }
            Index {
                files,
                ..Index::default()
            }
        })
    }
    fn touch(&self, id: &str) {
        if let Ok(file) = std::fs::File::options().write(true).open(self.path(id)) {
            let _ = file.set_modified(SystemTime::now());
        }
    }
    fn named_by_layouts(&self) -> BTreeSet<String> {
        self.layouts
            .everything()
            .iter()
            .flat_map(td_host::icons::asset_ids)
            .collect()
    }
}

#[cfg(test)]
mod quota {
    use super::*;

    fn icon(shade: u8) -> Vec<u8> {
        td_host::icon_pack::pack_any(&image::RgbaImage::from_pixel(
            64,
            64,
            image::Rgba([shade * 40, 0, 0, 255]),
        ))
    }
    const ICON: u64 = td_host::icon_pack::FILE_BYTES as u64;

    fn store(files: u64) -> (tempfile::TempDir, AssetStore, LayoutStore, Clock) {
        let dir = tempfile::tempdir().unwrap();
        let layouts = LayoutStore::new(dir.path().join("layouts"));
        let clock = Clock::default();
        let assets = AssetStore::new(
            dir.path().join("assets"),
            layouts.clone(),
            clock.clone(),
            files * ICON,
        );
        (dir, assets, layouts, clock)
    }

    fn layout_using(id: &str) -> Layout {
        Layout::parse(
            serde_json::json!({
                "schema":2,"name":"Kept","orientation":"landscape",
                "pages":[{"kind":"grid","id":"main","title":"Main","rows":1,"cols":1,
                "tiles":[{"id":"t0","label":"Kept","row":0,"col":0,"rowSpan":1,"colSpan":1,
                "icon":null,"asset":format!("sha256:{id}"),"action":{"type":"none"}}]}]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    }

    const LATER: Duration = Duration::from_secs(2 * 60 * 60);

    #[test]
    fn refuses_past_the_quota_when_nothing_can_go() {
        let (_dir, assets, _, _) = store(2);
        assets.put(&icon(1)).unwrap();
        assets.put(&icon(2)).unwrap();
        match assets.put(&icon(3)) {
            Err(PutError::Full { limit, actual }) => {
                assert_eq!((limit, actual), (2 * ICON, 3 * ICON));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            assets.put(&icon(1)).ok(),
            Some(td_link::asset::asset_id(&icon(1)))
        );
    }

    #[test]
    fn sweeps_the_oldest_orphan_and_keeps_layout_and_pinned_assets() {
        let (_dir, assets, layouts, clock) = store(3);
        let kept = assets.put(&icon(1)).unwrap();
        clock.advance(Duration::from_secs(1));
        let pinned = assets.put(&icon(2)).unwrap();
        clock.advance(Duration::from_secs(1));
        let orphan = assets.put(&icon(3)).unwrap();
        layouts.put("mock0001", &layout_using(&kept)).unwrap();
        let _pin = assets.pin([pinned.clone()]);
        clock.advance(LATER);
        let newest = assets.put(&icon(4)).unwrap();
        assert!(assets.get(&kept).is_some());
        assert!(assets.get(&pinned).is_some());
        assert!(assets.get(&orphan).is_none());
        assert!(assets.get(&newest).is_some());
        assert!(matches!(assets.put(&icon(5)), Err(PutError::Full { .. })));
    }

    #[test]
    fn a_dropped_pin_lets_its_asset_go() {
        let (_dir, assets, _, clock) = store(1);
        let first = assets.put(&icon(1)).unwrap();
        let pin = assets.pin([first.clone()]);
        clock.advance(LATER);
        assert!(matches!(assets.put(&icon(2)), Err(PutError::Full { .. })));
        drop(pin);
        assets.put(&icon(2)).unwrap();
        assert!(assets.get(&first).is_none());
    }

    #[test]
    fn recently_used_assets_survive_and_reads_count_as_use() {
        let (_dir, assets, _, clock) = store(2);
        let read = assets.put(&icon(1)).unwrap();
        let unread = assets.put(&icon(2)).unwrap();
        clock.advance(LATER);
        assets.get(&read).unwrap();
        assets.put(&icon(3)).unwrap();
        assert!(assets.get(&unread).is_none());
        assert!(matches!(assets.put(&icon(4)), Err(PutError::Full { .. })));
    }

    #[test]
    fn files_from_an_earlier_run_count_and_go_first() {
        let (dir, assets, layouts, clock) = store(2);
        let old = assets.put(&icon(1)).unwrap();
        let used = assets.put(&icon(2)).unwrap();
        let again = AssetStore::new(dir.path().join("assets"), layouts, clock.clone(), 2 * ICON);
        again.get(&used).unwrap();
        clock.advance(LATER);
        again.put(&icon(3)).unwrap();
        assert!(again.get(&old).is_none());
        assert!(again.get(&used).is_some());
    }

    #[test]
    fn startup_preserves_unrelated_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let id = "a".repeat(64);
        let names = [
            ".foo.tmp".to_owned(),
            format!(".{id}.bin.worker.tmp"),
            format!(".{id}.bin..tmp"),
            format!(".{id}.bin.4294967296.tmp"),
            format!(".{id}.other.123.tmp"),
            format!(".{}.bin.123.tmp", "g".repeat(64)),
        ];
        for name in &names {
            std::fs::write(dir.path().join(name), b"unrelated").unwrap();
        }
        let stale = dir.path().join(format!(".{id}.bin.123.tmp"));
        std::fs::write(&stale, b"partial asset").unwrap();
        let nested = dir.path().join(format!(".{id}.bin.456.tmp"));
        std::fs::create_dir(&nested).unwrap();
        let _assets = AssetStore::new(
            dir.path().to_owned(),
            LayoutStore::new(dir.path().join("layouts")),
            Clock::default(),
            ICON,
        );
        for name in &names {
            assert_eq!(std::fs::read(dir.path().join(name)).unwrap(), b"unrelated");
        }
        assert!(!stale.exists());
        assert!(nested.is_dir());
    }

    #[test]
    fn startup_sweeps_stale_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let assets_dir = dir.path().join("assets");
        std::fs::create_dir_all(&assets_dir).unwrap();
        let stale_a = assets_dir.join(format!(".{}.bin.999.tmp", "a".repeat(64)));
        let stale_b = assets_dir.join(format!(
            ".{}.bin.{}.tmp",
            "b".repeat(64),
            std::process::id()
        ));
        let cached_icon = assets_dir.join("0123456789abcdef.bin");
        let existing_bytes = icon(1);
        let existing_id = td_link::asset::asset_id(&existing_bytes);
        let existing_asset = assets_dir.join(format!("{existing_id}.bin"));
        std::fs::write(&stale_a, vec![0u8; 3 * ICON as usize]).unwrap();
        std::fs::write(&stale_b, vec![0u8; 2 * ICON as usize]).unwrap();
        std::fs::write(&cached_icon, &existing_bytes).unwrap();
        std::fs::write(&existing_asset, &existing_bytes).unwrap();

        let layouts = LayoutStore::new(dir.path().join("layouts"));
        let clock = Clock::default();
        let assets = AssetStore::new(assets_dir.clone(), layouts, clock, 2 * ICON);

        assert!(!stale_a.exists());
        assert!(!stale_b.exists());
        assert!(cached_icon.exists());
        assert!(existing_asset.exists());

        assets.put(&icon(2)).unwrap();
        let asset_bytes_on_disk: u64 = std::fs::read_dir(&assets_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path() != cached_icon)
            .map(|e| e.metadata().unwrap().len())
            .sum();
        assert_eq!(asset_bytes_on_disk, 2 * ICON);
    }
}
