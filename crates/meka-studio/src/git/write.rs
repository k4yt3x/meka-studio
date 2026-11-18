//! Staging and committing, as asked for by hand: git runs as from a terminal, the repository's
//! filters and hooks with it.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::process::Command;

use super::{Git, GitError, MAX_CHANGES, Output, run, run_with};

/// How long an answer waits for staging or committing, which a commit's hooks may make long: git
/// goes on after it, to its end.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The longest commit message taken.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

impl Git {
    /// Stages `paths`, each a path in the repository whose folder is `repo`, its names separated
    /// by `/`, or a folder in it, taken as written: what changed there, new and deleted files
    /// too. `.` is the whole repository.
    pub(crate) async fn stage(&self, repo: PathBuf, paths: &[String]) -> Result<(), GitError> {
        let input = pathspecs(paths)?;
        self.check(&repo).await?;
        let mut command = self.writing(Some(&repo), &[]);
        command.args([
            "--literal-pathspecs",
            "add",
            "-A",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ]);
        finished(&repo, run_to_end(command, input).await)
    }

    /// Unstages `paths`, as [`Self::stage`] takes them: the index takes them back as the last
    /// commit holds them, or before the first, lets go of them.
    pub(crate) async fn unstage(&self, repo: PathBuf, paths: &[String]) -> Result<(), GitError> {
        let input = pathspecs(paths)?;
        self.check(&repo).await?;
        let mut command = self.writing(Some(&repo), &[]);
        command.arg("--literal-pathspecs");
        if self.has_commit(&repo).await? {
            command.args(["restore", "--staged"]);
        } else {
            command.args(["rm", "--cached", "-r", "-q", "--ignore-unmatch"]);
        }
        command.args(["--pathspec-from-file=-", "--pathspec-file-nul"]);
        finished(&repo, run_to_end(command, input).await)
    }

    /// Commits what is staged, with `message`; with `tracked`, every change to a file git already
    /// tracks is staged first, as `git commit -a` does.
    pub(crate) async fn commit(
        &self,
        repo: PathBuf,
        message: &str,
        tracked: bool,
    ) -> Result<(), GitError> {
        if message.trim().is_empty() {
            return Err(GitError::BadRequest("a commit needs a message".to_owned()));
        }
        if message.len() > MAX_MESSAGE_BYTES {
            return Err(GitError::BadRequest(format!(
                "a commit message is at most {} KiB",
                MAX_MESSAGE_BYTES / 1024
            )));
        }
        self.check(&repo).await?;
        let mut command = self.writing(Some(&repo), &[]);
        command.args(["commit", "-q", "--file=-"]);
        if tracked {
            command.arg("--all");
        }
        finished(
            &repo,
            run_to_end(command, message.as_bytes().to_vec()).await,
        )
    }

    /// Whether the repository whose folder is `repo` has a commit checked out.
    async fn has_commit(&self, repo: &Path) -> Result<bool, GitError> {
        let mut command = self.reading(Some(repo), &[]);
        command.args(["rev-parse", "-q", "--verify", "HEAD^{commit}"]);
        let output = run(command)
            .await
            .map_err(|detail| GitError::Failed(repo.display().to_string(), detail))?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(GitError::Failed(repo.display().to_string(), output.error())),
        }
    }
}

/// Runs `command`, given `input`, to its end, apart from the request that asked for it: git
/// stopped part way, as dropping a request given up on would stop it, could leave the index
/// locked, or a commit's hooks half done. The answer waits for it at most [`WRITE_TIMEOUT`].
async fn run_to_end(mut command: Command, input: Vec<u8>) -> Result<Output, String> {
    command.kill_on_drop(false);
    let running = tokio::spawn(run_with(command, Some(input), None));
    match tokio::time::timeout(WRITE_TIMEOUT, running).await {
        Ok(ran) => ran.map_err(|error| error.to_string())?,
        Err(_) => Err(format!(
            "git is still at it after {} minutes, and goes on to its end",
            WRITE_TIMEOUT.as_secs() / 60
        )),
    }
}

/// `paths` as git reads them from its input, each ended with a NUL; refused where there are none,
/// too many, or one could name something outside the repository or break the list.
fn pathspecs(paths: &[String]) -> Result<Vec<u8>, GitError> {
    if paths.is_empty() || paths.len() > MAX_CHANGES * 2 {
        return Err(GitError::BadRequest(format!(
            "between 1 and {} paths are taken",
            MAX_CHANGES * 2
        )));
    }
    let mut input = Vec::new();
    for path in paths {
        if path.is_empty() || path.contains('\0') || path.starts_with('/') {
            return Err(GitError::BadRequest(format!(
                "{path} is not a path in a repository"
            )));
        }
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    Ok(input)
}

/// What became of git run in `repo` to write: done, or failed with what git said.
fn finished(repo: &Path, ran: Result<Output, String>) -> Result<(), GitError> {
    let failed = |detail: String| GitError::Failed(repo.display().to_string(), detail);
    let output = ran.map_err(failed)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failed(output.error()))
    }
}
