//! Native Brandfetch settings through the same session the future page will use.
use td_companion::{
    features::brandfetch::{ApiKey, BrandfetchError, BrandfetchSession, KeyTestResult},
    testing::InProcessCompanion,
};
use td_core::app_dirs::Dirs;

const KEY: &str = "fixture-private-brandfetch-value";

#[tokio::test]
async fn save_test_and_forget_both_values_without_exposing_the_key() {
    let mut td = InProcessCompanion::start().await;
    let folder = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        config: folder.path().join("config"),
    };
    let session = BrandfetchSession::new(td.handle.clone(), dirs.clone());
    let mut views = session.subscribe();
    session.load().await.unwrap();
    assert!(views.has_changed().unwrap());
    assert!(!session.view().key_set);
    assert!(!session.view().enabled);
    assert!(matches!(
        session.test_key().await,
        Err(BrandfetchError::NoKey)
    ));
    session
        .save_client_id("test-client-id".into())
        .await
        .unwrap();
    session.save_key(ApiKey::new(KEY.into())).await.unwrap();
    session.set_enabled(true).await.unwrap();
    let view = views.borrow_and_update().clone();
    assert_eq!(view.client_id.as_deref(), Some("test-client-id"));
    assert!(view.key_set && view.enabled);
    assert_eq!(view.test_result, KeyTestResult::NotTested);
    assert_eq!(view.logos, 0);
    assert!(view.expired.is_empty());
    assert!(!format!("{view:?}").contains(KEY));
    session.test_key().await.unwrap();
    assert_eq!(session.view().test_result, KeyTestResult::Passed);
    assert!(!format!("{:?}", session.view()).contains(KEY));
    assert!(!format!("{:?}", ApiKey::new(KEY.into())).contains(KEY));
    session.set_enabled(false).await.unwrap();
    assert!(session.view().key_set, "off keeps credentials");
    session.forget_key().await.unwrap();
    assert!(!session.view().key_set);
    assert_eq!(session.view().test_result, KeyTestResult::NotTested);
    session.forget_client_id().await.unwrap();
    let fresh = BrandfetchSession::new(td.handle.clone(), dirs);
    fresh.load().await.unwrap();
    let view = fresh.view();
    assert_eq!(view.client_id, None);
    assert!(!view.key_set && !view.enabled);
    assert!(!format!("{view:?}").contains(KEY));
    td.shutdown().await;
}

#[tokio::test]
async fn client_id_updates_preserve_unknown_fields_and_refuse_damaged_settings() {
    let td = td_companion::testing::TestCompanion::start().await;
    let dirs = Dirs {
        config: td.data_dir().join("config"),
    };
    let session = BrandfetchSession::new(td.handle(), dirs);
    session.save_client_id("first-id".into()).await.unwrap();
    let path = td.data_dir().join("brandfetch/settings.json");
    std::fs::write(&path, br#"{"client_id":"first-id","future":[1,2]}"#).unwrap();
    session.save_client_id(" second-id ".into()).await.unwrap();
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        written,
        serde_json::json!({"client_id":"second-id","future":[1,2]})
    );
    session.forget_client_id().await.unwrap();
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        written,
        serde_json::json!({"client_id":null,"future":[1,2]})
    );
    for damaged in ["{bad", "[]", r#"{"client_id":42}"#] {
        std::fs::write(&path, damaged).unwrap();
        assert!(matches!(
            session.save_client_id("valid-id".into()).await,
            Err(BrandfetchError::Store(_))
        ));
        assert!(matches!(
            session.forget_client_id().await,
            Err(BrandfetchError::Store(_))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), damaged);
    }
}

/// The server consumes the request and never answers. Its only exit is the client
/// closing the connection, or the test's failure deadline closing the fixture.
#[tokio::test]
async fn a_nonresponding_key_test_is_bounded_and_leaves_no_request_or_busy_view() {
    use std::time::{Duration, Instant};
    use tokio::{io::AsyncReadExt, net::TcpListener, sync::oneshot, time::timeout};

    let mut td = InProcessCompanion::start().await;
    let folder = tempfile::tempdir().unwrap();
    let session = BrandfetchSession::new(
        td.handle.clone(),
        Dirs {
            config: folder.path().join("config"),
        },
    );
    session.save_key(ApiKey::from(KEY)).await.unwrap();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    td.handle
        .brandfetch()
        .use_api(&format!("http://{}", listener.local_addr().unwrap()));
    let (received, started) = oneshot::channel();
    let server = tokio::spawn(async move {
        timeout(Duration::from_secs(25), async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 1024];
            assert!(socket.read(&mut bytes).await.unwrap() > 0);
            received.send(()).unwrap();
            // There is deliberately no response or timer that completes the request.
            loop {
                match socket.read(&mut bytes).await {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
                    Err(error) => panic!("unexpected socket error: {error}"),
                }
            }
        })
        .await
    });
    let began = Instant::now();
    let testing = tokio::spawn({
        let session = session.clone();
        async move { session.test_key().await }
    });
    // This test uses a single-thread runtime: receipt proves the caller is not blocked.
    timeout(Duration::from_secs(2), started)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.view().test_result, KeyTestResult::Testing);
    assert!(session.view().busy.is_some());
    assert!(!format!("{:?}", session.view()).contains(KEY));
    let result = testing.await.unwrap();
    let elapsed = began.elapsed();
    let closed = server.await.unwrap();
    assert!(
        closed.is_ok(),
        "the request socket must close without server intervention"
    );
    assert!(
        elapsed < Duration::from_secs(24),
        "key test exceeded its 20-second request budget: {elapsed:?}"
    );
    assert!(matches!(result, Err(BrandfetchError::Unavailable(_))));
    let view = session.view();
    assert_eq!(
        view.test_result,
        KeyTestResult::Failed(td_companion::features::brandfetch::Problem::Unavailable)
    );
    assert_eq!(view.busy, None);
    assert!(!format!("{view:?} {result:?}").contains(KEY));
    timeout(Duration::from_secs(2), session.forget_key())
        .await
        .unwrap()
        .unwrap();
    assert!(!session.view().key_set);
    td.shutdown().await;
}

#[tokio::test]
async fn cancelling_a_test_still_publishes_and_serializes_quick_switches_across_sessions() {
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
        time::timeout,
    };
    let mut td = InProcessCompanion::start().await;
    let folder = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        config: folder.path().join("config"),
    };
    let session = BrandfetchSession::new(td.handle.clone(), dirs.clone());
    let other = BrandfetchSession::new(td.handle.clone(), dirs);
    session.save_key(ApiKey::from(KEY)).await.unwrap();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    td.handle
        .brandfetch()
        .use_api(&format!("http://{}", listener.local_addr().unwrap()));
    let (received, started) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 1024];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        received.send(()).unwrap();
        released.await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .await
            .unwrap();
    });
    let testing = tokio::spawn({
        let session = session.clone();
        async move { session.test_key().await }
    });
    timeout(Duration::from_secs(2), started)
        .await
        .unwrap()
        .unwrap();
    testing.abort();
    assert!(testing.await.unwrap_err().is_cancelled());
    assert_eq!(session.view().test_result, KeyTestResult::Testing);

    let on = other.set_enabled(true);
    tokio::pin!(on);
    // The server checkpoint holds the shared operation gate after the caller is gone.
    assert!(timeout(Duration::from_millis(100), &mut on).await.is_err());
    let off = other.set_enabled(false);
    tokio::pin!(off);
    assert!(futures_util::poll!(off.as_mut()).is_pending());
    release.send(()).unwrap();
    timeout(Duration::from_secs(2), async {
        on.await.unwrap();
        off.await.unwrap();
    })
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(session.view().test_result, KeyTestResult::Passed);
    assert_eq!(session.view().busy, None);
    assert!(!other.view().enabled);
    session.load().await.unwrap();
    assert!(!session.view().enabled);
    session.forget_key().await.unwrap();
    assert!(!session.view().key_set);
    assert!(!format!("{:?} {:?}", session.view(), other.view()).contains(KEY));
    td.shutdown().await;
}

#[tokio::test]
async fn provider_errors_are_typed_and_never_echo_the_key_into_views_or_errors() {
    use td_companion::features::brandfetch::Problem;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let mut td = InProcessCompanion::start().await;
    let folder = tempfile::tempdir().unwrap();
    let session = BrandfetchSession::new(
        td.handle.clone(),
        Dirs {
            config: folder.path().join("config"),
        },
    );
    session.save_key(ApiKey::from(KEY)).await.unwrap();
    for (status, problem) in [
        (401, Problem::KeyRefused),
        (429, Problem::Quota),
        (500, Problem::Unavailable),
    ] {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        td.handle
            .brandfetch()
            .use_api(&format!("http://{}", listener.local_addr().unwrap()));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 2048];
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let read = socket.read(&mut bytes).await.unwrap();
                assert!(read > 0 && request.len() < 8192);
                request.extend_from_slice(&bytes[..read]);
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.contains("GET /v2/brands/brandfetch.com HTTP/1.1"));
            assert!(request.contains(&format!("Bearer {KEY}")));
            let response = format!(
                "HTTP/1.1 {status} Failed\r\nx-bf-error: {KEY}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{KEY}",
                KEY.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let error = session.test_key().await.unwrap_err();
        server.await.unwrap();
        let view = session.view();
        assert_eq!(view.test_result, KeyTestResult::Failed(problem));
        assert_eq!(view.problem, Some(problem));
        assert_eq!(view.busy, None);
        assert!(!format!("{view:?} {error:?} {error}").contains(KEY));
        assert!(view.key_set);
    }
    let error = session
        .save_key(ApiKey::from("fixture private value"))
        .await
        .unwrap_err();
    assert_eq!(session.view().problem, Some(Problem::InvalidInput));
    assert!(!format!("{:?} {error:?}", session.view()).contains("fixture private value"));
    assert_eq!(format!("{}", ApiKey::from(KEY)), "[redacted]");
    td.shutdown().await;
}

#[tokio::test]
async fn load_publishes_logo_counts_and_expiry_after_asset_changes() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use td_companion::{
        api::NoQuery,
        features::{
            assets::{Create, CreateAsset},
            brandfetch::{EXPIRE_AFTER_S, REFRESH_AFTER_S},
        },
        testing::TestCompanion,
    };
    let td = TestCompanion::start().await;
    let session = BrandfetchSession::new(
        td.handle(),
        Dirs {
            config: td.data_dir().join("config"),
        },
    );
    session.save_key(ApiKey::from(KEY)).await.unwrap();
    let asset: CreateAsset = serde_json::from_value(serde_json::json!({"source": {
        "kind": "brandfetch", "brand": "example.com", "logo_type": "icon", "theme": "auto",
        "padding": 4, "background": null, "tint": null
    }}))
    .unwrap();
    let job = td.call::<Create>((), NoQuery, asset).await.unwrap();
    let done = td.wait(&job).await;
    assert_eq!(done.state, td_companion::jobs::JobState::Succeeded);
    let id = done.result.unwrap()["id"].as_str().unwrap().to_owned();
    session.load().await.unwrap();
    assert_eq!(session.view().logos, 1);
    assert_eq!(session.view().need_refresh, 0);
    assert!(session.view().expired.is_empty());
    td.clock().advance(Duration::from_secs(REFRESH_AFTER_S + 1));
    session.load().await.unwrap();
    assert_eq!(session.view().need_refresh, 1);
    session.forget_key().await.unwrap();
    let expired_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + EXPIRE_AFTER_S
        + 1;
    td.refresh_brandfetch(expired_at).await;
    session.load().await.unwrap();
    let view = session.view();
    assert_eq!(view.logos, 1);
    assert_eq!(view.expired, vec![id]);
    assert!(!format!("{view:?}").contains(KEY));
}
