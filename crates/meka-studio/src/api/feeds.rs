//! A page's stream of feeds: opening it, and following and letting go of feeds on it. See
//! [`crate::feeds`].

use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    app::App,
    auth::Signed,
    error::ApiResult,
    feeds::{Follow, write},
    proxy::{accepts_gzip, batch_of, is_lean},
};

/// Opens the page's stream. Its every line is `data: <key> <line>`, a line of the feed followed
/// under that key as the feed sent it; the key `!` says `opened <stream>` first, then
/// `started <key>` as a feed starts, `ended <key>` as it ends, and
/// `failed <key> <status> <retry-after seconds, or -> <problem>` when it could not be followed.
#[utoipa::path(
    get,
    path = "/api/feeds",
    tag = "feeds",
    responses((status = 200, description = "The stream", content_type = "text/event-stream"), (status = 429, body = crate::error::Problem))
)]
pub(crate) async fn open(
    State(app): State<App>,
    signed: Signed,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let device = signed.session.id;
    let (name, _stream, receiver) = app.feeds.open(&device, is_lean(&headers))?;
    let gzip = accepts_gzip(&headers);
    let batch = batch_of(&headers);
    let (sender, body) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);
    tokio::spawn(async move {
        let ended = app.auth.until_ended(&device);
        write(&name, receiver, &sender, gzip, batch, ended).await;
        app.feeds.close(&name);
    });
    let mut response = Response::new(Body::from_stream(ReceiverStream::new(body)));
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    out.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    // Proxies that buffer, such as nginx, are told not to.
    out.insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    if gzip {
        out.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        out.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    }
    Ok(response.into_response())
}

/// Follows a feed on the stream `stream` under `key`, in place of what that key followed. How it
/// goes is said on the stream.
#[utoipa::path(
    put,
    path = "/api/feeds/{stream}/{key}",
    tag = "feeds",
    params(("stream" = String, Path), ("key" = String, Path)),
    request_body = Follow,
    responses((status = 204), (status = 400, body = crate::error::Problem), (status = 404, description = "No such stream is open", body = crate::error::Problem))
)]
pub(crate) async fn follow(
    State(app): State<App>,
    signed: Signed,
    Path((stream, key)): Path<(String, String)>,
    Json(follow): Json<Follow>,
) -> ApiResult<StatusCode> {
    app.feeds
        .find(&stream, &signed.session.id)?
        .follow(&app, &key, follow)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lets go of what `key` follows on the stream `stream`.
#[utoipa::path(
    delete,
    path = "/api/feeds/{stream}/{key}",
    tag = "feeds",
    params(("stream" = String, Path), ("key" = String, Path)),
    responses((status = 204), (status = 404, description = "No such stream is open", body = crate::error::Problem))
)]
pub(crate) async fn unfollow(
    State(app): State<App>,
    signed: Signed,
    Path((stream, key)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    app.feeds.find(&stream, &signed.session.id)?.unfollow(&key);
    Ok(StatusCode::NO_CONTENT)
}
