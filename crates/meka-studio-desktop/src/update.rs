//! Checking for a newer release: once a day while the setting is on, GitHub is asked for the
//! repository's latest release, and a newer one is told of once.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, bail};
use futures_util::StreamExt;
use meka_studio::desktop::Update;
use serde::Deserialize;
use tokio::time::Instant;

use crate::shell::Shell;

/// How long after starting the first check waits, so it does not slow the start.
const FIRST_CHECK: Duration = Duration::from_secs(60);

/// How long apart checks are.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a check may take.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// The most of GitHub's answer that is read.
const MAX_ANSWER_BYTES: usize = 1024 * 1024;

/// The fields of GitHub's latest release that are read.
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
}

/// Checks for newer releases for as long as the app runs.
pub(crate) async fn follow(shell: Arc<Shell>) {
    let client = match reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .user_agent(concat!("meka-studio-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::error!("could not make the client that checks for updates: {error}");
            return;
        }
    };
    let mut due = Instant::now() + FIRST_CHECK;
    loop {
        if shell.settings().check_updates && Instant::now() >= due {
            match latest(&client).await {
                Ok(version) => {
                    shell.found_release(is_newer(&version, env!("CARGO_PKG_VERSION")).then(|| {
                        Update {
                            url: format!("{}/releases/tag/{version}", env!("CARGO_PKG_REPOSITORY")),
                            version,
                        }
                    }))
                }
                Err(error) => tracing::warn!("could not check for a newer release: {error:#}"),
            }
            due = Instant::now() + CHECK_EVERY;
        }
        let wait = due
            .saturating_duration_since(Instant::now())
            .max(Duration::from_secs(1));
        tokio::select! {
            () = shell.settings_changed.notified() => {}
            () = tokio::time::sleep(wait) => {}
        }
    }
}

/// The version of the repository's latest release.
async fn latest(client: &reqwest::Client) -> anyhow::Result<String> {
    let url = env!("CARGO_PKG_REPOSITORY").replacen(
        "https://github.com/",
        "https://api.github.com/repos/",
        1,
    ) + "/releases/latest";
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk?);
        if body.len() > MAX_ANSWER_BYTES {
            bail!("GitHub's answer is larger than {MAX_ANSWER_BYTES} bytes");
        }
    }
    let release: Release =
        serde_json::from_slice(&body).context("GitHub's answer is not a release")?;
    if numbers(&release.tag_name).is_none() {
        bail!(
            "the latest release's tag {:?} is not a version",
            release.tag_name
        );
    }
    Ok(release.tag_name)
}

/// The numbers of a version such as `1.2.3`.
fn numbers(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
    let numbers = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(numbers)
}

/// Whether `candidate` is a later version than `current`.
fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((numbers(candidate), numbers(current)), (Some(candidate), Some(current)) if candidate > current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.44.0", "0.43.0"));
        assert!(is_newer("0.43.10", "0.43.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.43.0", "0.43.0"));
        assert!(!is_newer("0.42.9", "0.43.0"));
        assert!(
            !is_newer("v0.44.0", "0.43.0"),
            "a tag that is not a version"
        );
        assert!(!is_newer("0.44.0-rc.1", "0.43.0"));
        assert!(!is_newer("0.44", "0.43.0"));
    }
}
