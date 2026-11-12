//! The Companion serves the JSON API only. Every other path is a 404 that still passes the request guard and
//! carries the security headers.
use td_companion::testing::TestCompanion;

fn security_headers(response: &reqwest::Response, td: &TestCompanion) {
    for (key, value) in [
        ("x-frame-options", "DENY"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        assert_eq!(response.headers()[key], value);
    }
    let port = td.url.rsplit(':').next().unwrap();
    assert_eq!(
        response.headers()["content-security-policy"],
        format!(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' blob: data:; connect-src 'self' ws://127.0.0.1:{port}; \
         font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; \
         frame-ancestors 'none'"
        )
    );
}

async fn not_found(response: reqwest::Response, td: &TestCompanion) {
    assert_eq!(response.status(), 404);
    security_headers(&response, td);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "route.not_found"
    );
}

#[tokio::test]
async fn paths_outside_the_api_serve_nothing() {
    let td = TestCompanion::start().await;
    for path in [
        "/",
        "/index.html",
        "/devices/x",
        "/apiary",
        "/assets/index-12345678.js",
    ] {
        not_found(td.raw().get(path).send().await.unwrap(), &td).await;
    }
    for method in [
        reqwest::Method::HEAD,
        reqwest::Method::POST,
        reqwest::Method::PUT,
        reqwest::Method::DELETE,
    ] {
        let response = td.raw().request(method, "/devices/x").send().await.unwrap();
        assert_eq!(response.status(), 404);
        security_headers(&response, &td);
    }
}

#[tokio::test]
async fn paths_outside_the_api_pass_the_security_guard() {
    let td = TestCompanion::start().await;
    for path in ["/", "/devices/x"] {
        let response = td
            .raw()
            .get(path)
            .header("host", "evil.example:1")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 421);
        security_headers(&response, &td);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            "security.host"
        );
    }
    for (name, value) in [
        ("origin", "http://evil.example"),
        ("sec-fetch-site", "cross-site"),
    ] {
        let response = td.raw().get("/").header(name, value).send().await.unwrap();
        assert_eq!(response.status(), 403);
    }
}

#[tokio::test]
async fn unknown_api_paths_need_a_session_first() {
    let td = TestCompanion::start().await;
    for path in ["/api", "/api/unknown", "/api/v1/unknown"] {
        let response = td.raw().get(path).send().await.unwrap();
        assert_eq!(response.status(), 401);
        let response = td
            .raw()
            .get(path)
            .bearer_auth(&td.token)
            .send()
            .await
            .unwrap();
        not_found(response, &td).await;
    }
}
