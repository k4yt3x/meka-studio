//! In-process tests of the API, against a stand-in Meka on a local port.

use std::{
    io::Read,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::{
    app::{App, router},
    auth::password::Hasher,
    config::Config,
    store::Store,
};

const ORIGIN: &str = "http://localhost:4180";
const TOKEN: &str = "meka-test-token";
const PASSWORD: &str = "correct horse battery";

/// What the stand-in Meka saw of the last request to `/v1/echo`.
#[derive(Debug, Default, Clone)]
struct Seen {
    headers: Vec<(String, String)>,
}

/// A stand-in Meka: `/v1/info`, `/v1/echo`, the server feed and a session's event stream.
async fn stand_in_meka() -> (SocketAddr, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let app = Router::new()
        .route(
            "/v1/info",
            get(|headers: HeaderMap| async move {
                if headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    != Some(&format!("Bearer {TOKEN}"))
                {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                Json(serde_json::json!({
                    "version": "0.95.0",
                    "scopes": ["sessions:r", "sessions:w"],
                    "default_permission": "workspace",
                    "enabled_permissions": ["read", "workspace"],
                }))
                .into_response()
            }),
        )
        .route(
            "/v1/echo",
            any(
                |State(seen): State<Arc<Mutex<Seen>>>, headers: HeaderMap| async move {
                    let mut recorded: Vec<(String, String)> = headers
                        .iter()
                        .map(|(name, value)| {
                            (
                                name.to_string(),
                                value.to_str().unwrap_or_default().to_owned(),
                            )
                        })
                        .collect();
                    recorded.sort();
                    *seen.lock().unwrap() = Seen { headers: recorded };
                    Json(serde_json::json!({ "ok": true }))
                },
            ),
        )
        .route(
            "/v1/sessions/s1/feed",
            get(|| async {
                let old = (1..=50).map(|n| format!("line {n}\n")).collect::<String>();
                let new = old.replace("line 25\n", "line twenty-five\n");
                let completed = serde_json::json!({
                    "id": "toolu_1",
                    "is_error": false,
                    "content": [{"type": "text", "text": "Edited"}],
                    "change": {"path": "a.txt", "old": old, "new": new},
                });
                let first = format!("id: 1\nevent: tool_call.completed\ndata: {completed}\n\n");
                let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(first)])
                    .chain(futures_util::stream::pending());
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(stream))
                    .unwrap()
            }),
        )
        .route(
            "/v1/feed",
            get(|| async {
                let stream =
                    futures_util::stream::iter([Ok::<_, std::io::Error>("retry: 3000\n\n")])
                        .chain(futures_util::stream::pending());
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(stream))
                    .unwrap()
            }),
        )
        .route(
            "/v1/unauthorized",
            get(|| async { StatusCode::UNAUTHORIZED }),
        )
        .route(
            "/v1/sessions/resumed/feed",
            get(|uri: axum::http::Uri, headers: HeaderMap| async move {
                let from = headers
                    .get("last-event-id")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("-")
                    .to_owned();
                let said = format!("data: from {from} {}\n\n", uri.query().unwrap_or("-"));
                let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(said)])
                    .chain(futures_util::stream::pending());
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(stream))
                    .unwrap()
            }),
        )
        .route(
            "/v1/sessions/idle/feed",
            get(|| async {
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "type": "https://meka.bot/errors/session-not-loaded",
                        "status": 409,
                    })),
                )
            }),
        )
        // Anything else answers with the path and query it came to, as written.
        .fallback(|uri: axum::http::Uri, body: Body| async move {
            let bytes = body.collect().await.map_or(0, |body| body.to_bytes().len());
            Json(serde_json::json!({
                "path": uri.path(),
                "query": uri.query(),
                "bytes": bytes,
            }))
        })
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (address, seen)
}

/// Meka Studio in front of the stand-in Meka, with the user added, and its temporary folder.
async fn meka_studio() -> (App, Router, Arc<Mutex<Seen>>, tempfile::TempDir) {
    let (meka, seen) = stand_in_meka().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("token"), TOKEN).unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    let config = Config::parse(&format!(
        "listen = \"127.0.0.1:0\"\npublic_origin = \"{ORIGIN}\"\nstate_dir = '{0}/state'\n[meka]\nurl = \"http://{meka}\"\ntoken_file = '{0}/token'\n[push]\nsubject = \"mailto:me@example.com\"\n",
        dir.path().display()
    ))
    .unwrap();
    let store = Store::in_memory().unwrap();
    let hash = Hasher::new().unwrap().hash(PASSWORD).await.unwrap();
    store.add_user("me", &hash).await.unwrap();
    let app = App::open(config, store).await.unwrap();
    let router = router(&app).unwrap();
    (app, router, seen, dir)
}

fn request(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::ORIGIN, ORIGIN)
}

async fn send(router: &Router, request: Request<Body>) -> Response {
    router.clone().oneshot(request).await.unwrap()
}

async fn json(response: Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Signs in, returning the `Cookie` header to send.
async fn sign_in(router: &Router) -> String {
    let response = send(
        router,
        request("POST", "/api/auth/password")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "username": "me", "password": PASSWORD }).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"),
        "{cookie}"
    );
    cookie.split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn signing_in_sets_a_cookie_that_signs_requests_in() {
    let (_app, router, _, _dir) = meka_studio().await;
    let unsigned = send(
        &router,
        request("GET", "/api/auth/session")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);
    assert!(
        unsigned
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY),
        "every response carries the security headers"
    );
    let cookie = sign_in(&router).await;
    let response = send(
        &router,
        request("GET", "/api/auth/session")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let session = json(response).await;
    assert_eq!(session["username"], "me");
    assert_eq!(
        session["passkeys_available"], true,
        "localhost offers passkeys"
    );
    let out = send(
        &router,
        request("POST", "/api/auth/sign-out")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(out.status(), StatusCode::NO_CONTENT);
    let after = send(
        &router,
        request("GET", "/api/auth/session")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_write_from_another_site_is_refused() {
    let (_app, router, _, _dir) = meka_studio().await;
    let body =
        || Body::from(serde_json::json!({ "username": "me", "password": PASSWORD }).to_string());
    let elsewhere = Request::builder()
        .method("POST")
        .uri("/api/auth/password")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body())
        .unwrap();
    assert_eq!(
        send(&router, elsewhere).await.status(),
        StatusCode::FORBIDDEN
    );
    let unmarked = Request::builder()
        .method("POST")
        .uri("/api/auth/password")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body())
        .unwrap();
    assert_eq!(
        send(&router, unmarked).await.status(),
        StatusCode::FORBIDDEN
    );
    let same_origin = Request::builder()
        .method("POST")
        .uri("/api/auth/password")
        .header("sec-fetch-site", "same-origin")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body())
        .unwrap();
    assert_eq!(send(&router, same_origin).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn guessing_the_password_is_slowed() {
    let (_app, router, _, _dir) = meka_studio().await;
    let mut statuses = Vec::new();
    for _ in 0..12 {
        let response = send(
            &router,
            request("POST", "/api/auth/password")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "username": "me", "password": "wrong password" })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await;
        statuses.push(response.status());
    }
    assert_eq!(statuses[0], StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(*statuses.last().unwrap(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn requests_to_meka_carry_its_token_and_not_the_browsers_credentials() {
    let (_app, router, seen, _dir) = meka_studio().await;
    let unsigned = send(
        &router,
        request("GET", "/meka/v1/echo").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);
    let cookie = sign_in(&router).await;
    let response = send(
        &router,
        request("POST", "/meka/v1/echo?x=1")
            .header(header::COOKIE, &cookie)
            .header(header::AUTHORIZATION, "Bearer not-for-meka")
            .header("idempotency-key", "abc")
            .header("last-event-id", "7")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let seen = seen.lock().unwrap().clone();
    let header = |name: &str| {
        seen.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(
        header("authorization").as_deref(),
        Some("Bearer meka-test-token")
    );
    assert_eq!(header("cookie"), None);
    assert_eq!(header("idempotency-key").as_deref(), Some("abc"));
    assert_eq!(header("last-event-id").as_deref(), Some("7"));
    assert_eq!(header("origin"), None);
    let openapi = send(
        &router,
        request("GET", "/meka/v1/openapi.json")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(openapi.status(), StatusCode::NOT_FOUND);
}

/// Reads `body` into `text` until it holds `needle`, waiting at most five seconds.
async fn read_until(body: &mut Body, text: &mut String, needle: &str) {
    while !text.contains(needle) {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .unwrap_or_else(|_| panic!("no {needle:?} in {text}"))
            .unwrap()
            .unwrap();
        if let Ok(data) = frame.into_data() {
            text.push_str(&String::from_utf8_lossy(&data));
        }
    }
}

#[tokio::test]
async fn a_page_s_stream_carries_each_feed_under_its_key_until_it_lets_it_go() {
    let (app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let devices = send(
        &router,
        request("GET", "/api/account/devices")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let device = json(devices).await[0]["id"].as_str().unwrap().to_owned();
    let opened = send(
        &router,
        request("GET", "/api/feeds")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(opened.status(), StatusCode::OK);
    let mut body = opened.into_body();
    let mut text = String::new();
    read_until(&mut body, &mut text, "data: ! opened ").await;
    read_until(&mut body, &mut text, "\n\n").await;
    let stream = text
        .split("data: ! opened ")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let follow = |key: &str, cookie: &str, follow: serde_json::Value| {
        request("PUT", &format!("/api/feeds/{stream}/{key}"))
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(follow.to_string()))
            .unwrap()
    };
    let feed = serde_json::json!({"kind": "feed", "path": "/v1/sessions/s1/feed", "attend": true});
    assert_eq!(
        send(&router, follow("s1.0", &cookie, feed.clone()))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    read_until(&mut body, &mut text, "data: ! started s1.0\n").await;
    // The feed's own lines, its change made a diff as on the way through.
    read_until(
        &mut body,
        &mut text,
        "data: s1.0 event: tool_call.completed\n",
    )
    .await;
    read_until(&mut body, &mut text, "-line 25\\n+line twenty-five\\n").await;
    assert!(text.contains("data: s1.0 id: 1\n"), "{text}");
    assert!(!text.contains("\"old\""));
    assert!(app.watching.is_watching(&device, "s1"));
    // A feed Meka refuses is said to have failed, with Meka's problem.
    let idle = serde_json::json!({"kind": "feed", "path": "/v1/sessions/idle/feed"});
    send(&router, follow("idle", &cookie, idle)).await;
    read_until(&mut body, &mut text, "data: ! failed idle 409 - {").await;
    assert!(text.contains("session-not-loaded"), "{text}");
    // A feed resumes from its cursor, and asks Meka whether the page attends.
    let resumed = serde_json::json!({
        "kind": "feed", "path": "/v1/sessions/resumed/feed", "cursor": "41", "attend": false,
    });
    send(&router, follow("r", &cookie, resumed)).await;
    read_until(&mut body, &mut text, "data: r data: from 41 attend=false\n").await;
    let broken = serde_json::json!({
        "kind": "feed", "path": "/v1/sessions/resumed/feed", "cursor": "4\n1",
    });
    assert_eq!(
        send(&router, follow("b", &cookie, broken)).await.status(),
        StatusCode::BAD_REQUEST
    );
    // Only Meka's feeds, under keys of a few plain characters.
    let turns = serde_json::json!({"kind": "feed", "path": "/v1/sessions/s1/turns"});
    assert_eq!(
        send(&router, follow("t", &cookie, turns)).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&router, follow("a%20b", &cookie, feed.clone()))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    // Another sign-in finds no such stream.
    let other = sign_in(&router).await;
    assert_eq!(
        send(&router, follow("s1.1", &other, feed)).await.status(),
        StatusCode::NOT_FOUND
    );
    // Let go of, a session's feed no longer marks its device as watching.
    let unfollowed = send(
        &router,
        request("DELETE", &format!("/api/feeds/{stream}/s1.0"))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(unfollowed.status(), StatusCode::NO_CONTENT);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!app.watching.is_watching(&device, "s1"));
    // The page leaving closes its stream.
    drop(body);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let idle = serde_json::json!({"kind": "feed", "path": "/v1/feed"});
    assert_eq!(
        send(&router, follow("server", &cookie, idle))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_device_saving_data_hears_at_once_that_a_feed_started() {
    let (_app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let opened = send(
        &router,
        request("GET", "/api/feeds")
            .header(header::COOKIE, &cookie)
            .header("x-meka-studio-batch", "2000")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let mut body = opened.into_body();
    let mut text = String::new();
    read_until(&mut body, &mut text, "\n\n").await;
    let stream = text
        .strip_prefix("data: ! opened ")
        .unwrap()
        .trim()
        .to_owned();
    let followed = send(
        &router,
        request("PUT", &format!("/api/feeds/{stream}/server"))
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"kind": "feed", "path": "/v1/feed"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(followed.status(), StatusCode::NO_CONTENT);
    // The feed's own lines wait for the batch; that it started goes ahead of them.
    let mut frames = Vec::new();
    for _ in 0..2 {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        frames.push(String::from_utf8_lossy(&frame.into_data().unwrap()).into_owned());
    }
    assert_eq!(frames[0], "data: ! started server\n\n");
    assert!(
        frames[1].contains("data: server retry: 3000\n"),
        "{frames:?}"
    );
}

#[tokio::test]
async fn a_page_s_compressed_stream_ends_whole_with_its_sign_in() {
    let (_app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let opened = send(
        &router,
        request("GET", "/api/feeds")
            .header(header::COOKIE, &cookie)
            .header(header::ACCEPT_ENCODING, "gzip")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(opened.headers()[header::CONTENT_ENCODING], "gzip");
    let mut body = opened.into_body();
    let mut compressed = Vec::new();
    let first = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    compressed.extend_from_slice(&first.into_data().unwrap());
    let out = send(
        &router,
        request("POST", "/api/auth/sign-out")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(out.status(), StatusCode::NO_CONTENT);
    let rest = tokio::time::timeout(Duration::from_secs(5), body.collect())
        .await
        .unwrap()
        .unwrap()
        .to_bytes();
    compressed.extend_from_slice(&rest);
    // A stream cut short would leave the gzip stream without its end, which reading refuses.
    let mut text = String::new();
    flate2::read::GzDecoder::new(&compressed[..])
        .read_to_string(&mut text)
        .unwrap();
    assert!(text.starts_with("data: ! opened "), "{text}");
}

#[tokio::test]
async fn an_event_stream_arrives_compressed_with_its_change_as_a_diff_and_marks_its_watcher() {
    let (app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let devices = send(
        &router,
        request("GET", "/api/account/devices")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let device = json(devices).await[0]["id"].as_str().unwrap().to_owned();
    let response = send(
        &router,
        request("GET", "/meka/v1/sessions/s1/feed")
            .header(header::COOKIE, &cookie)
            .header(header::ACCEPT_ENCODING, "gzip, br")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_ENCODING).unwrap(),
        "gzip"
    );
    assert!(app.watching.is_watching(&device, "s1"));
    let mut body = response.into_body();
    let mut compressed = Vec::new();
    let mut text = String::new();
    // The first event comes on its own, though the stream stays open.
    while !text.contains("\n\n") {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Ok(data) = frame.into_data() {
            compressed.extend_from_slice(&data);
        }
        text.clear();
        let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
        let mut partial = Vec::new();
        // A stream cut mid-way reads up to where it stands, then errs.
        let mut buffer = [0_u8; 4096];
        while let Ok(read) = decoder.read(&mut buffer) {
            if read == 0 {
                break;
            }
            partial.extend_from_slice(&buffer[..read]);
        }
        text = String::from_utf8_lossy(&partial).into_owned();
    }
    assert!(text.contains("-line 25\\n+line twenty-five\\n"), "{text}");
    assert!(!text.contains("\"old\""));
    drop(body);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!app.watching.is_watching(&device, "s1"));
}

#[tokio::test]
async fn a_server_feed_from_a_page_in_front_marks_its_device_while_open() {
    let (app, router, seen, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let devices = send(
        &router,
        request("GET", "/api/account/devices")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let device = json(devices).await[0]["id"].as_str().unwrap().to_owned();
    let feed = |focused: bool| {
        let mut builder = request("GET", "/meka/v1/feed").header(header::COOKIE, &cookie);
        if focused {
            builder = builder.header("x-meka-studio-focused", "1");
        }
        send(&router, builder.body(Body::empty()).unwrap())
    };
    let behind = feed(false).await;
    assert_eq!(behind.status(), StatusCode::OK);
    assert!(!app.watching.is_focused(&device));
    let front = feed(true).await;
    assert_eq!(front.status(), StatusCode::OK);
    assert!(app.watching.is_focused(&device));
    drop(front);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!app.watching.is_focused(&device));
    drop(behind);
    // The header is Meka Studio's; Meka never sees it.
    send(
        &router,
        request("GET", "/meka/v1/echo")
            .header(header::COOKIE, &cookie)
            .header("x-meka-studio-focused", "1")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let recorded = seen.lock().unwrap().headers.clone();
    assert!(
        recorded
            .iter()
            .all(|(name, _)| name != "x-meka-studio-focused")
    );
}

#[tokio::test]
async fn risky_changes_need_a_recent_confirmation() {
    let (app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let challenge = || {
        request("POST", "/api/account/passkeys/challenge")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(&router, challenge()).await.status(),
        StatusCode::OK,
        "signing in confirms"
    );
    let user = app.store.user().await.unwrap().unwrap();
    let session = app.store.sessions(user.id).await.unwrap().remove(0);
    app.store
        .age_verification_for_test(&session.id)
        .await
        .unwrap();
    let refused = send(&router, challenge()).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        json(refused).await["type"],
        "urn:meka-studio:verification-required"
    );
    let confirm = send(
        &router,
        request("POST", "/api/auth/confirm")
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "password": PASSWORD }).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(confirm.status(), StatusCode::OK);
    assert_eq!(send(&router, challenge()).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn files_list_and_uploads_land_through_the_api() {
    let (_app, router, _, dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let work = dir.path().join("work");
    let encoded = |path: std::path::PathBuf| {
        percent_encoding::utf8_percent_encode(
            &path.display().to_string(),
            percent_encoding::NON_ALPHANUMERIC,
        )
        .to_string()
    };
    let start = send(
        &router,
        request("POST", "/api/files/uploads")
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "dir": work, "name": "a.txt", "size": 5 }).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(start.status(), StatusCode::OK);
    let id = json(start).await["id"].as_str().unwrap().to_owned();
    let chunk = send(
        &router,
        request("PUT", &format!("/api/files/uploads/{id}/chunk?offset=0"))
            .header(header::COOKIE, &cookie)
            .body(Body::from("hello"))
            .unwrap(),
    )
    .await;
    assert_eq!(chunk.status(), StatusCode::OK);
    let finish = send(
        &router,
        request("POST", &format!("/api/files/uploads/{id}/finish"))
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(finish.status(), StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("work/a.txt")).unwrap(),
        "hello"
    );
    let raw = send(
        &router,
        request(
            "GET",
            &format!("/api/files/raw?path={}", encoded(work.join("a.txt"))),
        )
        .header(header::COOKIE, &cookie)
        .header(header::RANGE, "bytes=1-3")
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(raw.status(), StatusCode::PARTIAL_CONTENT);
    assert!(
        raw.headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("sandbox")
    );
    let bytes = raw.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"ell");
    std::fs::write(
        dir.path().join("work/page.html"),
        "<script>alert(1)</script>",
    )
    .unwrap();
    let html = send(
        &router,
        request(
            "GET",
            &format!("/api/files/raw?path={}", encoded(work.join("page.html"))),
        )
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(
        html.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/octet-stream"
    );
    assert!(
        html.headers()
            .get(header::CONTENT_DISPOSITION)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    // Anywhere the user may read, by a path from home too.
    let listing = send(
        &router,
        request(
            "GET",
            &format!("/api/files/list?path={}", encoded(work.join("x/../."))),
        )
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(listing.status(), StatusCode::OK);
    let listing = json(listing).await;
    assert_eq!(listing["path"], work.display().to_string());
    assert_eq!(
        listing["home"],
        crate::files::resolve("~").unwrap().display().to_string()
    );
    let names: Vec<&str> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["a.txt", "page.html"]);
    let relative = send(
        &router,
        request("GET", "/api/files/list?path=work")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(relative.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_listing_names_its_repository_and_git_says_how_that_stands() {
    let (app, router, _, dir) = meka_studio().await;
    if !app.git.available().await {
        return;
    }
    let cookie = sign_in(&router).await;
    let work = dir.path().join("work");
    let global = dir.path().join("gitconfig");
    std::fs::write(&global, "").unwrap();
    let made = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&work)
        .env("GIT_CONFIG_GLOBAL", &global)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(made.status.success());
    std::fs::create_dir(work.join("inner")).unwrap();
    std::fs::write(work.join("inner/new.txt"), "").unwrap();
    let encoded = |path: &std::path::Path| {
        percent_encoding::utf8_percent_encode(
            &path.display().to_string(),
            percent_encoding::NON_ALPHANUMERIC,
        )
        .to_string()
    };
    let get = |path: String| {
        request("GET", &path)
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap()
    };
    let listing = json(
        send(
            &router,
            get(format!(
                "/api/files/list?path={}",
                encoded(&work.join("inner"))
            )),
        )
        .await,
    )
    .await;
    assert_eq!(listing["repo"], work.display().to_string());
    let outside = json(
        send(
            &router,
            get(format!("/api/files/list?path={}", encoded(dir.path()))),
        )
        .await,
    )
    .await;
    assert_eq!(outside["repo"], serde_json::Value::Null);
    let status = send(
        &router,
        get(format!("/api/git/status?repo={}", encoded(&work))),
    )
    .await;
    assert_eq!(status.status(), StatusCode::OK);
    let status = json(status).await;
    assert_eq!(status["branch"], "main");
    assert_eq!(status["changes"][0]["path"], "inner/");
    assert_eq!(status["changes"][0]["kind"], "untracked");
    let not_one = send(
        &router,
        get(format!(
            "/api/git/status?repo={}",
            encoded(&work.join("inner"))
        )),
    )
    .await;
    assert_eq!(not_one.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_file_reads_as_the_commit_asked_for_holds_it() {
    let (app, router, _, dir) = meka_studio().await;
    if !app.git.available().await {
        return;
    }
    let cookie = sign_in(&router).await;
    let work = dir.path().join("work");
    let global = dir.path().join("gitconfig");
    std::fs::write(&global, "").unwrap();
    let git = |args: &[&str]| {
        let done = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.org"])
            .args(args)
            .current_dir(&work)
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(done.status.success());
        String::from_utf8(done.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(work.join("a b.txt"), "one\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "first"]);
    let commit = git(&["rev-parse", "HEAD"]);
    std::fs::write(work.join("a b.txt"), "two\n").unwrap();
    let get = |commit: &str, path: &str| {
        let encoded = |text: &str| {
            percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        };
        request(
            "GET",
            &format!(
                "/api/git/file?repo={}&commit={}&path={}",
                encoded(&work.display().to_string()),
                encoded(commit),
                encoded(path)
            ),
        )
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap()
    };
    let read = send(&router, get(&commit, "a b.txt")).await;
    assert_eq!(read.status(), StatusCode::OK);
    let read = json(read).await;
    assert_eq!(read["text"], "one\n");
    assert_eq!(read["size"], 4);
    assert_eq!(read["binary"], false);
    assert_eq!(read["too_large"], false);
    let missing = send(&router, get(&commit, "new.txt")).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(json(missing).await["type"], "urn:meka-studio:not-committed");
    let named = send(&router, get("HEAD", "a b.txt")).await;
    assert_eq!(named.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn changes_are_staged_committed_and_read_back_from_the_history() {
    let (app, router, _, dir) = meka_studio().await;
    if !app.git.available().await {
        return;
    }
    let cookie = sign_in(&router).await;
    let work = dir.path().join("work");
    let global = dir.path().join("gitconfig");
    std::fs::write(&global, "").unwrap();
    let made = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&work)
        .env("GIT_CONFIG_GLOBAL", &global)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(made.status.success());
    // Meka Studio's git reads the user's own config too: the repository's own says who commits,
    // and keeps their signing and hooks out of the test.
    let hooks = dir.path().join("no-hooks");
    for (key, value) in [
        ("user.name", "t"),
        ("user.email", "t@example.org"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", hooks.to_str().unwrap()),
    ] {
        let set = std::process::Command::new("git")
            .args(["config", "--local", key, value])
            .current_dir(&work)
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(set.status.success());
    }
    std::fs::write(work.join("a.txt"), "one\ntwo\n").unwrap();
    let repo = work.display().to_string();
    let encoded = |text: &str| {
        percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC).to_string()
    };
    let get = |path: String| {
        request("GET", &path)
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap()
    };
    let post = |path: &str, body: serde_json::Value| {
        request("POST", path)
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let stat = json(
        send(
            &router,
            get(format!("/api/git/diffstat?repo={}", encoded(&repo))),
        )
        .await,
    )
    .await;
    assert_eq!(stat["files"][0]["path"], "a.txt");
    assert_eq!(stat["files"][0]["added"], 2);
    let staged = send(
        &router,
        post(
            "/api/git/stage",
            serde_json::json!({ "repo": repo, "paths": ["a.txt"] }),
        ),
    )
    .await;
    assert_eq!(staged.status(), StatusCode::NO_CONTENT);
    let status = json(
        send(
            &router,
            get(format!("/api/git/status?repo={}", encoded(&repo))),
        )
        .await,
    )
    .await;
    assert_eq!(status["changes"][0]["staged"], "all");
    let committed = send(
        &router,
        post(
            "/api/git/commit",
            serde_json::json!({ "repo": repo, "message": "first" }),
        ),
    )
    .await;
    let code = committed.status();
    assert_eq!(code, StatusCode::NO_CONTENT, "{}", json(committed).await);
    let status = json(
        send(
            &router,
            get(format!("/api/git/status?repo={}", encoded(&repo))),
        )
        .await,
    )
    .await;
    let head = status["commit"].as_str().unwrap().to_owned();
    let history = json(
        send(
            &router,
            get(format!(
                "/api/git/history?repo={}&head={head}",
                encoded(&repo)
            )),
        )
        .await,
    )
    .await;
    assert_eq!(history["commits"][0]["subject"], "first");
    assert_eq!(history["more"], false);
    let changes = json(
        send(
            &router,
            get(format!(
                "/api/git/commit?repo={}&commit={head}",
                encoded(&repo)
            )),
        )
        .await,
    )
    .await;
    assert_eq!(changes["files"][0]["kind"], "added");
    assert_eq!(changes["files"][0]["added"], 2);
    let nothing = send(
        &router,
        post(
            "/api/git/commit",
            serde_json::json!({ "repo": repo, "message": "again" }),
        ),
    )
    .await;
    assert_eq!(nothing.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn folders_followed_tell_of_their_changes_until_the_sign_in_ends() {
    let (_app, router, _, dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let work = dir.path().join("work");
    std::fs::create_dir(work.join("inner")).unwrap();
    let opened = send(
        &router,
        request("GET", "/api/feeds")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(
        opened.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let mut body = opened.into_body();
    let mut seen = String::new();
    read_until(&mut body, &mut seen, "data: ! opened ").await;
    read_until(&mut body, &mut seen, "\n\n").await;
    let stream = seen
        .split("data: ! opened ")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let follow = |key: &str, paths: serde_json::Value| {
        request("PUT", &format!("/api/feeds/{stream}/{key}"))
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "kind": "folders", "paths": paths }).to_string(),
            ))
            .unwrap()
    };
    // Following starts once its folders are watched, as the page lists them again then: a change
    // as soon as it hears so is told, however many folders came before.
    let mut crowd: Vec<String> = (0..40)
        .map(|i| {
            let folder = work.join(format!("d{i}"));
            std::fs::create_dir(&folder).unwrap();
            folder.display().to_string()
        })
        .collect();
    let early = work.join("early");
    std::fs::create_dir(&early).unwrap();
    crowd.push(early.display().to_string());
    send(&router, follow("early", serde_json::json!(crowd))).await;
    read_until(&mut body, &mut seen, "data: ! started early\n").await;
    std::fs::write(early.join("x"), "x").unwrap();
    read_until(
        &mut body,
        &mut seen,
        &format!(
            "data: early event: change\ndata: early data: {}",
            early.display()
        ),
    )
    .await;
    let too_many: Vec<String> = (0..65).map(|i| format!("{}/{i}", work.display())).collect();
    assert_eq!(
        send(&router, follow("many", serde_json::json!(too_many)))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let missing = work.join("missing").display().to_string();
    let inner = work.join("inner").display().to_string();
    let followed = send(
        &router,
        follow(
            "f",
            serde_json::json!([work.display().to_string(), inner, missing, "not/absolute"]),
        ),
    )
    .await;
    assert_eq!(followed.status(), StatusCode::NO_CONTENT);
    read_until(
        &mut body,
        &mut seen,
        &format!("data: f event: gone\ndata: f data: {missing}"),
    )
    .await;
    read_until(
        &mut body,
        &mut seen,
        "data: f event: unwatched\ndata: f data: not/absolute",
    )
    .await;
    // Folders with nothing left to follow stay followed, to be tried again by following them anew
    // rather than at once.
    send(&router, follow("lone", serde_json::json!([missing]))).await;
    read_until(&mut body, &mut seen, "data: lone event: gone").await;
    std::fs::write(work.join("inner").join("new.txt"), "x").unwrap();
    read_until(
        &mut body,
        &mut seen,
        &format!("data: f event: change\ndata: f data: {inner}"),
    )
    .await;
    std::fs::remove_dir_all(work.join("inner")).unwrap();
    read_until(
        &mut body,
        &mut seen,
        &format!("data: f event: gone\ndata: f data: {inner}"),
    )
    .await;
    std::fs::write(work.join("new.txt"), "x").unwrap();
    read_until(
        &mut body,
        &mut seen,
        &format!("data: f event: change\ndata: f data: {}", work.display()),
    )
    .await;
    assert!(!seen.contains("ended lone"), "{seen}");
    let out = send(
        &router,
        request("POST", "/api/auth/sign-out")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(out.status(), StatusCode::NO_CONTENT);
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(_)) = body.frame().await {}
    })
    .await;
    assert!(ended.is_ok(), "the stream ends with the sign-in");
}

#[tokio::test]
async fn entries_chosen_download_as_one_zip_by_the_id_kept_for_them() {
    let (_app, router, _, dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let work = dir.path().join("work");
    std::fs::write(work.join("a.txt"), "alpha").unwrap();
    std::fs::write(work.join("b.txt"), "beta").unwrap();
    std::fs::write(work.join("c.txt"), "gamma").unwrap();
    let select = |paths: serde_json::Value| {
        request("POST", "/api/files/zip/selection")
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "paths": paths }).to_string(),
            ))
            .unwrap()
    };
    let made = send(
        &router,
        select(serde_json::json!([work.join("a.txt"), work.join("c.txt")])),
    )
    .await;
    assert_eq!(made.status(), StatusCode::OK);
    let id = json(made).await["id"].as_str().unwrap().to_owned();
    let download = |query: String| {
        request("GET", &format!("/api/files/zip?{query}"))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap()
    };
    let zip = send(&router, download(format!("selection={id}"))).await;
    assert_eq!(zip.status(), StatusCode::OK);
    let archive = zip.into_body().collect().await.unwrap().to_bytes();
    let archive = zip::ZipArchive::new(std::io::Cursor::new(archive.to_vec())).unwrap();
    let mut names: Vec<String> = archive
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["work/", "work/a.txt", "work/c.txt"]);
    // A path is absolute, and an id is one given.
    let refused = send(&router, select(serde_json::json!(["a.txt"]))).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let empty = send(&router, select(serde_json::json!([]))).await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    let unknown = send(&router, download("selection=nothing".to_owned())).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn text_saved_through_the_api_lands_only_over_what_was_read() {
    let (_app, router, _, dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let path = dir.path().join("work").join("notes.md");
    std::fs::write(&path, "one").unwrap();
    let encoded = percent_encoding::utf8_percent_encode(
        &path.display().to_string(),
        percent_encoding::NON_ALPHANUMERIC,
    )
    .to_string();
    let read = send(
        &router,
        request("GET", &format!("/api/files/text?path={encoded}&edit=true"))
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let version = json(read).await["version"].as_str().unwrap().to_owned();
    let save = |text: &str, version: Option<&str>| {
        request("PUT", "/api/files/text")
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "path": path, "text": text, "version": version }).to_string(),
            ))
            .unwrap()
    };
    // Larger than the API's other bodies may be.
    let long = "x".repeat(512 * 1024);
    let saved = send(&router, save(&long, Some(&version))).await;
    assert_eq!(saved.status(), StatusCode::OK);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), long);
    let stale = send(&router, save("two", Some(&version))).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(json(stale).await["type"], "urn:meka-studio:changed");
    let there = send(&router, save("two", None)).await;
    assert_eq!(there.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn paths_reach_meka_as_written_and_never_leave_its_api() {
    let (_app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let get = |path: &str| {
        request("GET", path)
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap()
    };
    let response = send(&router, get("/meka/v1/memories/a%2Fb%3Fc%25d?x=1")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let seen = json(response).await;
    assert_eq!(seen["path"], "/v1/memories/a%2Fb%3Fc%25d");
    assert_eq!(seen["query"], "x=1");
    for path in [
        "/meka/v1/x/../openapi.json",
        "/meka/v1/x/%2E%2e/openapi.json",
        "/meka/v1/%2e/openapi.json",
    ] {
        assert_eq!(
            send(&router, get(path)).await.status(),
            StatusCode::BAD_REQUEST,
            "{path}"
        );
    }
    for path in [
        "/meka/v1/openapi.json",
        "/meka/v1/openapi%2Ejson",
        "/meka/v2/x",
    ] {
        assert_eq!(
            send(&router, get(path)).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}

#[tokio::test]
async fn a_body_larger_than_meka_takes_is_refused() {
    let (_app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let post = |bytes: usize| {
        request("POST", "/meka/v1/import")
            .header(header::COOKIE, &cookie)
            .body(Body::from(vec![b'x'; bytes]))
            .unwrap()
    };
    let response = send(&router, post(1024)).await;
    assert_eq!(json(response).await["bytes"], 1024);
    assert_eq!(
        send(&router, post(10 * 1024 * 1024 + 1)).await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn meka_refusing_its_token_does_not_sign_the_browser_out() {
    let (_app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let response = send(
        &router,
        request("GET", "/meka/v1/unauthorized")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        json(response).await["type"],
        "urn:meka-studio:meka-unavailable"
    );
}

#[tokio::test]
async fn signing_a_device_out_ends_its_open_streams() {
    let (_app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let response = send(
        &router,
        request("GET", "/meka/v1/sessions/s1/feed")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    assert!(body.next().await.is_some(), "the first event arrives");
    let out = send(
        &router,
        request("POST", "/api/auth/sign-out")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(out.status(), StatusCode::NO_CONTENT);
    assert!(out.headers().contains_key("clear-site-data"));
    let ended = tokio::time::timeout(Duration::from_secs(5), async {
        while body.next().await.is_some() {}
    })
    .await;
    assert!(ended.is_ok(), "the stream ends with the sign-in");
}

#[tokio::test]
async fn changing_the_password_takes_the_current_one_and_signs_every_other_device_out() {
    let (_app, router, _, _dir) = meka_studio().await;
    let mine = sign_in(&router).await;
    let other = sign_in(&router).await;
    let change = |current: &str| {
        request("PUT", "/api/account/password")
            .header(header::COOKIE, &mine)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "current": current, "password": "another long password" })
                    .to_string(),
            ))
            .unwrap()
    };
    let wrong = send(&router, change("not the password")).await;
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
    assert_eq!(json(wrong).await["type"], "urn:meka-studio:wrong-password");
    let response = send(&router, change(PASSWORD)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["count"], 1);
    let session = |cookie: &str| {
        request("GET", "/api/auth/session")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(&router, session(&other)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(send(&router, session(&mine)).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn passkey_challenges_are_limited_per_network() {
    let (_app, router, _, _dir) = meka_studio().await;
    let challenge = || {
        request("POST", "/api/auth/passkey/challenge")
            .body(Body::empty())
            .unwrap()
    };
    for _ in 0..20 {
        assert_eq!(send(&router, challenge()).await.status(), StatusCode::OK);
    }
    let refused = send(&router, challenge()).await;
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(refused.headers().contains_key(header::RETRY_AFTER));
}

/// A stand-in desktop app: its settings, the notifications and folders it was asked to show, the
/// zoom steps it took, and how often it was asked to quit.
#[derive(Default)]
struct StandInHost {
    settings: Mutex<crate::desktop::DesktopSettings>,
    shown: Mutex<Vec<crate::desktop::Notification>>,
    opened: Mutex<Vec<crate::desktop::Folder>>,
    zoomed: Mutex<Vec<crate::desktop::ZoomStep>>,
    quits: Mutex<usize>,
}

impl crate::desktop::Host for StandInHost {
    fn notify(&self, notification: crate::desktop::Notification) {
        self.shown.lock().unwrap().push(notification);
    }

    fn state(&self) -> crate::desktop::DesktopState {
        crate::desktop::DesktopState {
            settings: *self.settings.lock().unwrap(),
            meka: crate::desktop::MekaProcess {
                state: crate::desktop::ProcessState::Running,
                pid: Some(1),
                last_exit: None,
            },
            update: None,
            logs: "/logs".to_owned(),
            portable: false,
        }
    }

    fn save_settings(&self, settings: crate::desktop::DesktopSettings) -> anyhow::Result<()> {
        *self.settings.lock().unwrap() = settings;
        Ok(())
    }

    fn restart_meka(&self) {}

    fn open_folder(&self, folder: crate::desktop::Folder) -> anyhow::Result<()> {
        self.opened.lock().unwrap().push(folder);
        Ok(())
    }

    fn quit(&self) {
        *self.quits.lock().unwrap() += 1;
    }

    fn zoom(&self, step: crate::desktop::ZoomStep) -> anyhow::Result<()> {
        self.zoomed.lock().unwrap().push(step);
        Ok(())
    }

    fn toggle_full_screen(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Meka Studio inside a stand-in desktop app, on `store`: the app, the router, the window's
/// `Cookie` header, and the folder holding its state.
async fn desktop_studio(
    store: Store,
    host: Arc<StandInHost>,
) -> (App, Router, String, tempfile::TempDir) {
    let (meka, _) = stand_in_meka().await;
    let dir = tempfile::tempdir().unwrap();
    let config = Config::desktop(
        "127.0.0.1:41234".parse().unwrap(),
        dir.path().join("state"),
        format!("http://{meka}").parse().unwrap(),
        TOKEN.to_owned(),
    )
    .unwrap();
    let (app, ticket) = App::open_desktop(config, store, host).await.unwrap();
    let router = router(&app).unwrap();
    let enter = |ticket: &str| {
        Request::builder()
            .uri(format!("/api/desktop/enter?ticket={ticket}"))
            .body(Body::empty())
            .unwrap()
    };
    let wrong = send(&router, enter("not-the-ticket")).await;
    assert_eq!(wrong.status(), StatusCode::SEE_OTHER);
    assert!(!wrong.headers().contains_key(header::SET_COOKIE));
    let entered = send(&router, enter(&ticket)).await;
    assert_eq!(entered.status(), StatusCode::SEE_OTHER);
    assert_eq!(entered.headers()[header::LOCATION], "/");
    let cookie = entered.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let again = send(&router, enter(&ticket)).await;
    assert!(
        !again.headers().contains_key(header::SET_COOKIE),
        "a ticket signs in once"
    );
    (app, router, cookie, dir)
}

fn from_the_window(method: &str, path: &str, cookie: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::ORIGIN, "http://127.0.0.1:41234")
        .header(header::COOKIE, cookie)
}

#[tokio::test]
async fn the_desktop_window_is_signed_in_and_has_nothing_of_signing_in_to_manage() {
    let host = Arc::new(StandInHost::default());
    let (_app, router, cookie, _dir) =
        desktop_studio(Store::in_memory().unwrap(), Arc::clone(&host)).await;
    let session = send(
        &router,
        from_the_window("GET", "/api/auth/session", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(session.status(), StatusCode::OK);
    assert_eq!(json(session).await["desktop"], true);
    for (method, path) in [
        ("POST", "/api/auth/sign-out"),
        ("POST", "/api/auth/password"),
        ("POST", "/api/auth/passkey/challenge"),
        ("GET", "/api/account/devices"),
        ("PUT", "/api/account/password"),
        ("GET", "/api/push"),
    ] {
        let refused = send(
            &router,
            from_the_window(method, path, &cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND, "{method} {path}");
        assert_eq!(
            json(refused).await["type"],
            "urn:meka-studio:disabled",
            "{method} {path}"
        );
    }
    let still = send(
        &router,
        from_the_window("GET", "/api/auth/session", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(still.status(), StatusCode::OK, "signing out was refused");
    let saved = send(
        &router,
        from_the_window("PUT", "/api/desktop/settings", &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"close":"quit","notifications":false,"check_updates":false}"#,
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::OK);
    assert_eq!(json(saved).await["settings"]["close"], "quit");
    assert_eq!(
        host.settings.lock().unwrap().close,
        crate::desktop::CloseAction::Quit
    );
    let tested = send(
        &router,
        from_the_window("POST", "/api/desktop/notifications/test", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(tested.status(), StatusCode::NO_CONTENT);
    assert_eq!(host.shown.lock().unwrap().len(), 1);
    for folder in ["data", "home", "..%2F..%2Fetc"] {
        send(
            &router,
            from_the_window(
                "POST",
                &format!("/api/desktop/folders/{folder}/open"),
                &cookie,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    }
    assert_eq!(
        *host.opened.lock().unwrap(),
        [crate::desktop::Folder::Data],
        "only the app's own folders open, by name"
    );
    for (step, status) in [
        (r#"{"step":"in"}"#, StatusCode::NO_CONTENT),
        (r#"{"step":"sideways"}"#, StatusCode::UNPROCESSABLE_ENTITY),
        (r#"{"step":"reset"}"#, StatusCode::NO_CONTENT),
    ] {
        let zoomed = send(
            &router,
            from_the_window("POST", "/api/desktop/window/zoom", &cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(step))
                .unwrap(),
        )
        .await;
        assert_eq!(zoomed.status(), status, "{step}");
    }
    assert_eq!(*host.zoomed.lock().unwrap(), [
        crate::desktop::ZoomStep::In,
        crate::desktop::ZoomStep::Reset
    ]);
    let quitting = send(
        &router,
        from_the_window("POST", "/api/desktop/quit", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(quitting.status(), StatusCode::ACCEPTED);
    assert_eq!(*host.quits.lock().unwrap(), 1);
}

#[tokio::test]
async fn each_start_signs_the_desktop_window_in_afresh() {
    let store = Store::in_memory().unwrap();
    let (_first, _, before, _first_dir) =
        desktop_studio(store.clone(), Arc::new(StandInHost::default())).await;
    let (_second, router, after, _second_dir) =
        desktop_studio(store, Arc::new(StandInHost::default())).await;
    let session = |cookie: &str| {
        from_the_window("GET", "/api/auth/session", cookie)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(&router, session(&before)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&router, session(&after)).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn only_the_desktop_app_has_its_settings() {
    let (_app, router, _, _dir) = meka_studio().await;
    let cookie = sign_in(&router).await;
    let response = send(
        &router,
        request("GET", "/api/desktop")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let session = send(
        &router,
        request("GET", "/api/auth/session")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(json(session).await["desktop"], false);
}

#[tokio::test]
async fn the_desktop_window_stays_signed_in_however_long_it_goes_unused() {
    let (app, router, cookie, _dir) = desktop_studio(
        Store::in_memory().unwrap(),
        Arc::new(StandInHost::default()),
    )
    .await;
    app.store.age_sessions_for_test().await.unwrap();
    app.sweep().await;
    let session = send(
        &router,
        from_the_window("GET", "/api/auth/session", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(session.status(), StatusCode::OK);
}
