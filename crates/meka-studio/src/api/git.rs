//! git's endpoints: how a repository Files shows stands, to mark its entries and show its branch;
//! the lines each change adds and removes; a file as a commit holds it, to compare with what is on
//! disk; staging and committing; and the history.

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::{
    app::App,
    auth::Signed,
    error::ApiResult,
    files::resolve,
    git::{CommitChanges, CommittedFile, DiffStat, GitStatus, History},
};

/// A repository, by its folder, as a listing names it.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct Repository {
    pub(crate) repo: String,
}

/// How the repository whose folder is `repo` stands: its branch, and the entries changed, new or
/// ignored since its last commit. Read again for each request, by running git, which runs no
/// program the repository's own config names.
#[utoipa::path(
    get,
    path = "/api/git/status",
    tag = "git",
    params(Repository),
    responses((status = 200, body = GitStatus), (status = 400, body = crate::error::Problem), (status = 404, body = crate::error::Problem), (status = 422, body = crate::error::Problem))
)]
pub(crate) async fn status(
    State(app): State<App>,
    _signed: Signed,
    Query(query): Query<Repository>,
) -> ApiResult<Json<GitStatus>> {
    let repo = resolve(&query.repo)?;
    Ok(Json(GitStatus::clone(&*app.git.status(repo).await?)))
}

/// A file in a repository, as a commit holds it.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct CommittedQuery {
    /// The repository's folder, as a listing names it.
    pub(crate) repo: String,
    /// The commit's full id, as the repository's status names it.
    pub(crate) commit: String,
    /// Where the file is in the repository, its names separated by `/`, as a change names it.
    pub(crate) path: String,
}

/// The file `path` as the commit `commit` of the repository whose folder is `repo` holds it, to
/// compare with what is on disk: its text, up to 10 MiB. A path the commit holds no file at, or a
/// folder, a symlink or a submodule, is 404 with the problem type `not-committed`. Read by running
/// git, which reads only its own objects and fetches none.
#[utoipa::path(
    get,
    path = "/api/git/file",
    tag = "git",
    params(CommittedQuery),
    responses((status = 200, body = CommittedFile), (status = 400, body = crate::error::Problem), (status = 404, body = crate::error::Problem), (status = 422, body = crate::error::Problem))
)]
pub(crate) async fn file(
    State(app): State<App>,
    _signed: Signed,
    Query(query): Query<CommittedQuery>,
) -> ApiResult<Json<CommittedFile>> {
    let repo = resolve(&query.repo)?;
    Ok(Json(
        app.git.committed(repo, &query.commit, &query.path).await?,
    ))
}

/// The lines each change since the last commit of the repository whose folder is `repo` adds and
/// removes, staged or not, new files' too. A binary file's, or one past 10 MiB, are not counted.
/// Read by running git, which runs no program the repository's own config names.
#[utoipa::path(
    get,
    path = "/api/git/diffstat",
    tag = "git",
    params(Repository),
    responses((status = 200, body = DiffStat), (status = 400, body = crate::error::Problem), (status = 404, body = crate::error::Problem), (status = 422, body = crate::error::Problem))
)]
pub(crate) async fn diff_stat(
    State(app): State<App>,
    _signed: Signed,
    Query(query): Query<Repository>,
) -> ApiResult<Json<DiffStat>> {
    let repo = resolve(&query.repo)?;
    Ok(Json(app.git.diff_stat(repo).await?))
}

/// Paths in a repository to stage or unstage.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct Paths {
    /// The repository's folder, as a listing names it.
    pub(crate) repo: String,
    /// Each a path in the repository, its names separated by `/`, or a folder in it, taken as
    /// written; `.` is the whole repository.
    pub(crate) paths: Vec<String>,
}

/// Stages what changed at `paths`, new and deleted files too, as `git add -A` does: with the
/// filters the repository names, as from a terminal.
#[utoipa::path(
    post,
    path = "/api/git/stage",
    tag = "git",
    request_body = Paths,
    responses((status = 204), (status = 400, body = crate::error::Problem), (status = 404, body = crate::error::Problem), (status = 422, body = crate::error::Problem))
)]
pub(crate) async fn stage(
    State(app): State<App>,
    _signed: Signed,
    Json(body): Json<Paths>,
) -> ApiResult<StatusCode> {
    let repo = resolve(&body.repo)?;
    app.git.stage(repo, &body.paths).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unstages `paths`: the index takes them back as the last commit holds them.
#[utoipa::path(
    post,
    path = "/api/git/unstage",
    tag = "git",
    request_body = Paths,
    responses((status = 204), (status = 400, body = crate::error::Problem), (status = 404, body = crate::error::Problem), (status = 422, body = crate::error::Problem))
)]
pub(crate) async fn unstage(
    State(app): State<App>,
    _signed: Signed,
    Json(body): Json<Paths>,
) -> ApiResult<StatusCode> {
    let repo = resolve(&body.repo)?;
    app.git.unstage(repo, &body.paths).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A commit to make.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct NewCommit {
    /// The repository's folder, as a listing names it.
    pub(crate) repo: String,
    pub(crate) message: String,
    /// Stage every change to a file git already tracks first, as `git commit -a` does.
    #[serde(default)]
    pub(crate) tracked: bool,
}

/// Commits what is staged, as from a terminal: the repository's hooks run, and the commit is
/// signed if the user's config says so. git runs to its end, however long its hooks take; past five
/// minutes the answer is a problem saying so. What a hook or git says when it fails is the
/// problem's detail.
#[utoipa::path(
    post,
    path = "/api/git/commit",
    tag = "git",
    request_body = NewCommit,
    responses((status = 204), (status = 400, body = crate::error::Problem), (status = 404, body = crate::error::Problem), (status = 422, body = crate::error::Problem))
)]
pub(crate) async fn commit(
    State(app): State<App>,
    _signed: Signed,
    Json(body): Json<NewCommit>,
) -> ApiResult<StatusCode> {
    let repo = resolve(&body.repo)?;
    app.git.commit(repo, &body.message, body.tracked).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A page of a branch's history.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct HistoryQuery {
    /// The repository's folder, as a listing names it.
    pub(crate) repo: String,
    /// The commit the history goes back from, by its full id, as the repository's status names
    /// it: the same for every page.
    pub(crate) head: String,
    /// How many commits the pages before this one held.
    #[serde(default)]
    pub(crate) skip: usize,
}

/// The commits from `head` back, newest first, 50 at a time.
#[utoipa::path(
    get,
    path = "/api/git/history",
    tag = "git",
    params(HistoryQuery),
    responses((status = 200, body = History), (status = 400, body = crate::error::Problem), (status = 404, body = crate::error::Problem), (status = 422, body = crate::error::Problem))
)]
pub(crate) async fn history(
    State(app): State<App>,
    _signed: Signed,
    Query(query): Query<HistoryQuery>,
) -> ApiResult<Json<History>> {
    let repo = resolve(&query.repo)?;
    Ok(Json(app.git.history(repo, &query.head, query.skip).await?))
}

/// A commit, by its full id.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct CommitQuery {
    /// The repository's folder, as a listing names it.
    pub(crate) repo: String,
    pub(crate) commit: String,
}

/// What the commit changed from its first parent, or from nothing for a first commit: each file
/// with the lines it added and removed, and the commit's whole message.
#[utoipa::path(
    get,
    path = "/api/git/commit",
    tag = "git",
    params(CommitQuery),
    responses((status = 200, body = CommitChanges), (status = 400, body = crate::error::Problem), (status = 404, body = crate::error::Problem), (status = 422, body = crate::error::Problem))
)]
pub(crate) async fn commit_changes(
    State(app): State<App>,
    _signed: Signed,
    Query(query): Query<CommitQuery>,
) -> ApiResult<Json<CommitChanges>> {
    let repo = resolve(&query.repo)?;
    Ok(Json(app.git.commit_changes(repo, &query.commit).await?))
}
