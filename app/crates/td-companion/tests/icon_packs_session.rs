use td_companion::{icon_packs::IconPacksSession, testing::TestCompanion};
use td_host::{
    catalog::{Error, Fetcher},
    testing::{listed, tarball},
};

const ICONS: &[u8] = br#"{"prefix":"mono","icons":{"star":{"body":"<path d=\"M0 0h16v16z\"/>"}}}"#;
struct Registry(Vec<u8>);
impl Fetcher for Registry {
    fn fetch(&self, _: &str) -> Result<Vec<u8>, Error> {
        Ok(self.0.clone())
    }
}
async fn start() -> TestCompanion {
    let archive = tarball(&[("package/icons.json", ICONS)]);
    TestCompanion::start_with_packs(vec![listed("mono", &archive)], Box::new(Registry(archive)))
        .await
}
#[tokio::test]
async fn session_installs_and_publishes_the_list() {
    let td = start().await;
    let session = IconPacksSession::new(td.handle().icon_packs());
    let view = session.subscribe();
    session.load().await.unwrap();
    assert!(!view.borrow().packs[0].installed);
    session.install("mono").await.unwrap();
    assert!(view.borrow().packs[0].installed);
    assert!(!view.borrow().busy);
    assert!(view.borrow().error.is_none());
}

#[tokio::test]
async fn session_repeated_failure_and_retry_report_each_outcome() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Repairable {
        good: Vec<u8>,
        broken: Arc<AtomicBool>,
    }
    impl Fetcher for Repairable {
        fn fetch(&self, _: &str) -> Result<Vec<u8>, Error> {
            Ok(if self.broken.load(Ordering::SeqCst) {
                vec![0]
            } else {
                self.good.clone()
            })
        }
    }
    let archive = tarball(&[("package/icons.json", ICONS)]);
    let broken = Arc::new(AtomicBool::new(true));
    let td = TestCompanion::start_with_packs(
        vec![listed("mono", &archive)],
        Box::new(Repairable {
            good: archive,
            broken: broken.clone(),
        }),
    )
    .await;
    let session = IconPacksSession::new(td.handle().icon_packs());
    for _ in 0..2 {
        assert!(session.install("mono").await.is_err());
        assert!(session.view().error.is_some());
        assert!(!session.view().packs[0].installed);
        assert!(!session.view().busy);
    }
    broken.store(false, Ordering::SeqCst);
    session.install("mono").await.unwrap();
    assert!(session.view().packs[0].installed);
    assert!(session.view().error.is_none());
}

#[tokio::test]
async fn session_removes_an_installed_pack() {
    let td = start().await;
    let session = IconPacksSession::new(td.handle().icon_packs());
    session.install("mono").await.unwrap();
    session.remove("mono").await.unwrap();
    assert!(!session.view().packs[0].installed);
    assert!(session.remove("mono").await.is_err());
}

#[tokio::test]
async fn session_imports_iconify_and_reports_invalid_files() {
    let td = start().await;
    let session = IconPacksSession::new(td.handle().icon_packs());
    assert!(session.import(b"invalid".to_vec()).await.is_err());
    let bytes = String::from_utf8(ICONS.to_vec())
        .unwrap()
        .replace("mono", "mine")
        .into_bytes();
    let pack = session.import(bytes).await.unwrap();
    assert_eq!(pack.origin, td_host::packs::PackOrigin::Imported);
    assert!(
        session
            .view()
            .packs
            .iter()
            .any(|p| p.prefix == "mine" && p.installed)
    );
    assert!(session.view().error.is_none());
}

#[tokio::test]
async fn session_switch_persists_without_removing_packs() {
    let td = start().await;
    let service = td.handle().icon_packs();
    let session = IconPacksSession::new(service.clone());
    session.load().await.unwrap();
    assert_eq!(session.view().enabled, Some(true));
    session.install("mono").await.unwrap();
    let (off, on) = tokio::join!(session.set_enabled(false), session.set_enabled(true));
    off.unwrap();
    on.unwrap();
    assert_eq!(session.view().enabled, Some(true));
    session.set_enabled(false).await.unwrap();
    let reopened = IconPacksSession::new(service);
    reopened.load().await.unwrap();
    assert_eq!(reopened.view().enabled, Some(false));
    assert!(reopened.view().packs[0].installed);
}

#[tokio::test]
async fn held_download_keeps_the_caller_responsive_and_finishes_after_awaiter_drops() {
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;
    struct Held {
        archive: Vec<u8>,
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl Fetcher for Held {
        fn fetch(&self, _: &str) -> Result<Vec<u8>, Error> {
            self.entered
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            Ok(self.archive.clone())
        }
    }
    let archive = tarball(&[("package/icons.json", ICONS)]);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let td = TestCompanion::start_with_packs(
        vec![listed("mono", &archive)],
        Box::new(Held {
            archive,
            entered: Mutex::new(Some(entered_tx)),
            release: Mutex::new(release_rx),
        }),
    )
    .await;
    let session = IconPacksSession::new(td.handle().icon_packs());
    let installer = session.clone();
    let task = tokio::spawn(async move { installer.install("mono").await });
    tokio::time::timeout(Duration::from_secs(5), entered_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(session.view().busy);
    assert_eq!(
        session.view().stage,
        Some(td_host::packs::PackStage::Download)
    );
    task.abort();
    let _ = task.await;
    let off = session.set_enabled(false);
    let on = session.set_enabled(true);
    tokio::pin!(off, on);
    assert!(futures_util::poll!(&mut off).is_pending());
    assert!(futures_util::poll!(&mut on).is_pending());
    release_tx.send(()).unwrap();
    off.await.unwrap();
    on.await.unwrap();
    assert_eq!(session.view().enabled, Some(true));
    // This queues behind the worker, including its final publication.
    session.remove("mono").await.unwrap();
    assert!(!session.view().busy);
    assert!(!session.view().packs[0].installed);
}
