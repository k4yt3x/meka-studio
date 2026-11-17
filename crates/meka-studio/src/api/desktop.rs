//! The desktop app's settings and the Meka it runs, which only Meka Studio inside the app has.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::{
    app::App,
    auth::Signed,
    desktop::{Desktop, DesktopSettings, DesktopState, Host, Notification},
    error::{ApiError, ApiResult},
};

/// The desktop app this server runs in, or the refusal of a server that runs on its own.
fn desktop(app: &App) -> ApiResult<&Desktop> {
    app.desktop
        .as_ref()
        .ok_or_else(|| ApiError::disabled("Only the desktop app has these."))
}

/// What the desktop app this server runs in does for it; see [`desktop`].
fn host(app: &App) -> ApiResult<&Arc<dyn Host>> {
    Ok(&desktop(app)?.host)
}

/// The ticket the desktop app opens its window with.
#[derive(Debug, Deserialize)]
pub(crate) struct EnterQuery {
    pub(crate) ticket: String,
}

#[utoipa::path(
    get,
    path = "/api/desktop/enter",
    tag = "desktop",
    params(("ticket" = String, Query, description = "The ticket the app opened the window with")),
    responses(
        (status = 303, description = "Signed in, if the ticket was not used yet; on to the interface"),
        (status = 404, description = "Not the desktop app", body = crate::error::Problem)
    )
)]
pub(crate) async fn enter(
    State(app): State<App>,
    Query(query): Query<EnterQuery>,
) -> ApiResult<Response> {
    // A ticket used already goes on all the same: the window holds the sign-in it was for.
    Ok(match desktop(&app)?.redeem(&query.ticket) {
        Some(token) => (StatusCode::SEE_OTHER, [
            (header::LOCATION, "/".to_owned()),
            (header::SET_COOKIE, app.auth.session_cookie(&token)),
        ])
            .into_response(),
        None => (StatusCode::SEE_OTHER, [(header::LOCATION, "/")]).into_response(),
    })
}

#[utoipa::path(
    get,
    path = "/api/desktop",
    tag = "desktop",
    responses(
        (status = 200, body = DesktopState),
        (status = 404, description = "Not the desktop app", body = crate::error::Problem)
    )
)]
pub(crate) async fn state(
    State(app): State<App>,
    _signed: Signed,
) -> ApiResult<Json<DesktopState>> {
    Ok(Json(host(&app)?.state()))
}

#[utoipa::path(
    put,
    path = "/api/desktop/settings",
    tag = "desktop",
    request_body = DesktopSettings,
    responses(
        (status = 200, description = "Saved; how the app stands now", body = DesktopState),
        (status = 404, description = "Not the desktop app", body = crate::error::Problem)
    )
)]
pub(crate) async fn save_settings(
    State(app): State<App>,
    _signed: Signed,
    Json(settings): Json<DesktopSettings>,
) -> ApiResult<Json<DesktopState>> {
    let host = host(&app)?;
    host.save_settings(settings)?;
    Ok(Json(host.state()))
}

#[utoipa::path(
    post,
    path = "/api/desktop/meka/restart",
    tag = "desktop",
    responses(
        (status = 202, description = "Meka is stopping, to start again"),
        (status = 404, description = "Not the desktop app", body = crate::error::Problem)
    )
)]
pub(crate) async fn restart_meka(State(app): State<App>, _signed: Signed) -> ApiResult<StatusCode> {
    host(&app)?.restart_meka();
    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(
    post,
    path = "/api/desktop/logs/open",
    tag = "desktop",
    responses(
        (status = 204, description = "The folder is open in the file manager"),
        (status = 404, description = "Not the desktop app", body = crate::error::Problem)
    )
)]
pub(crate) async fn open_logs(State(app): State<App>, _signed: Signed) -> ApiResult<StatusCode> {
    host(&app)?.open_logs()?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/desktop/notifications/test",
    tag = "desktop",
    responses(
        (status = 204, description = "A test notification is shown"),
        (status = 404, description = "Not the desktop app", body = crate::error::Problem)
    )
)]
pub(crate) async fn test_notification(
    State(app): State<App>,
    _signed: Signed,
) -> ApiResult<StatusCode> {
    host(&app)?.notify(Notification {
        title: "Meka Studio".to_owned(),
        body: crate::push::PushKind::Test.words().to_owned(),
        session: None,
    });
    Ok(StatusCode::NO_CONTENT)
}
