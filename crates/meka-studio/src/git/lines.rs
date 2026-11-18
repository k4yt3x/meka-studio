//! The lines each change since the last commit adds and removes, as `git diff --numstat` counts
//! them, and as a new file holds them.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use serde::Serialize;
use utoipa::ToSchema;

use super::{ChangeKind, Git, GitError, MAX_CHANGES, TIMEOUT, run, run_with};
use crate::files;

/// Past this size a file counts as binary, its lines not counted, so one huge file changed does
/// not hold the rest up: as much as the editor takes.
pub(super) const BIG_FILE: &str = "10m";

/// The most new files whose lines are counted, each read whole.
const MAX_NEW_FILES: usize = 1000;

/// The most of the new files read to count their lines, all together: past it, those left are
/// not counted.
const MAX_NEW_BYTES: u64 = 32 * 1024 * 1024;

/// The lines each change since the last commit adds and removes.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DiffStat {
    /// Each file changed, new ones too, by its path as the status names it.
    pub(crate) files: Vec<FileStat>,
    /// Whether more changed than are told: past 10,000, none are.
    pub(crate) too_many: bool,
}

/// The lines a file's change adds and removes: none for a binary file, or one past 10 MiB.
#[derive(Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct FileStat {
    pub(crate) path: String,
    pub(crate) added: Option<u64>,
    pub(crate) removed: Option<u64>,
}

impl Git {
    /// The lines each change since the last commit of the repository whose folder is `repo`
    /// adds and removes, staged or not: tracked files' as git counts them, with what the
    /// repository names to run blanked as for its status, and new files' all added. A folder new
    /// as a whole is not looked into.
    pub(crate) async fn diff_stat(&self, repo: PathBuf) -> Result<DiffStat, GitError> {
        let status = self.status(repo.clone()).await?;
        if status.too_many {
            return Ok(DiffStat {
                files: Vec::new(),
                too_many: true,
            });
        }
        let failed = |detail: String| GitError::Failed(repo.display().to_string(), detail);
        let base = match &status.commit {
            Some(commit) => commit.clone(),
            None => self.empty_tree(&repo).await?,
        };
        let mut config = self.programs_to_blank(&repo).await?;
        config.push(("core.bigFileThreshold".to_owned(), BIG_FILE.to_owned()));
        let mut command = self.reading(Some(&repo), &config);
        command.args([
            "diff",
            "--numstat",
            "-z",
            "--no-textconv",
            "--no-ext-diff",
            "-M",
        ]);
        command.args([base.as_str(), "--"]);
        let output = run(command).await.map_err(failed)?;
        if !output.capped && !output.status.success() {
            return Err(failed(output.error()));
        }
        let mut files = parse_numstat(&output.stdout);
        if output.capped || files.len() > MAX_CHANGES {
            return Ok(DiffStat {
                files: Vec::new(),
                too_many: true,
            });
        }
        let new: Vec<String> = status
            .changes
            .iter()
            .filter(|change| change.kind == ChangeKind::Untracked && !change.path.ends_with('/'))
            .take(MAX_NEW_FILES)
            .map(|change| change.path.clone())
            .collect();
        let folder = repo.clone();
        let counted = tokio::task::spawn_blocking(move || count_new(&folder, new, MAX_NEW_BYTES))
            .await
            .map_err(|error| failed(error.to_string()))?;
        files.extend(counted);
        Ok(DiffStat {
            files,
            too_many: false,
        })
    }

    /// The id of the empty tree, as the repository's object format writes it, to compare with
    /// before its first commit.
    async fn empty_tree(&self, repo: &Path) -> Result<String, GitError> {
        let failed = |detail: String| GitError::Failed(repo.display().to_string(), detail);
        let mut command = self.reading(Some(repo), &[]);
        command.args(["hash-object", "-t", "tree", "--stdin"]);
        let output = run_with(command, Some(Vec::new()), Some(TIMEOUT))
            .await
            .map_err(failed)?;
        if !output.status.success() {
            return Err(failed(output.error()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}

/// The files `git diff --numstat -z` tells of in `output`: each its lines added and removed, a
/// tab, then its path, or for a rename a tab, then where it was and where it is, apart. A binary
/// file's counts are `-`.
pub(super) fn parse_numstat(output: &[u8]) -> Vec<FileStat> {
    let mut files = Vec::new();
    let mut records = output.split(|byte| *byte == 0);
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(record);
        let mut fields = text.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            // Renamed: where it was, then where it is.
            records.next();
            records
                .next()
                .map(|to| String::from_utf8_lossy(to).into_owned())
                .unwrap_or_default()
        } else {
            path.to_owned()
        };
        files.push(FileStat {
            path,
            added: added.parse().ok(),
            removed: removed.parse().ok(),
        });
    }
    files
}

/// The lines of each of the new files `paths` in `folder`, all added, reading at most `budget`
/// bytes of them together: one past what is left of it is not counted.
fn count_new(folder: &Path, paths: Vec<String>, mut budget: u64) -> Vec<FileStat> {
    paths
        .into_iter()
        .map(|path| {
            let lines = count_lines(&folder.join(&path), &mut budget);
            FileStat {
                path,
                added: lines,
                removed: lines.map(|_| 0),
            }
        })
        .collect()
}

/// How many lines the file at `path` holds, taking what is read of it from `budget`; none for one
/// that is not a regular file, looks binary, or is longer than the editor takes or `budget` has
/// left.
fn count_lines(path: &Path, budget: &mut u64) -> Option<u64> {
    let (file, metadata) = files::open_regular(path).ok()?;
    let most = files::MAX_EDIT_BYTES.min(*budget);
    if metadata.len() > most {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(most + 1).read_to_end(&mut bytes).ok()?;
    let read = bytes.len() as u64;
    if read > most {
        return None;
    }
    *budget -= read;
    if files::looks_binary(&bytes) {
        return None;
    }
    let ended = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
    Some(ended + u64::from(bytes.last().is_some_and(|byte| *byte != b'\n')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_files_are_counted_within_what_may_be_read_of_them() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "1\n2\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "3\n4\n5").unwrap();
        std::fs::write(dir.path().join("c.txt"), "6\n").unwrap();
        let counted = count_new(
            dir.path(),
            vec!["a.txt".to_owned(), "b.txt".to_owned(), "c.txt".to_owned()],
            6,
        );
        let lines: Vec<Option<u64>> = counted.iter().map(|file| file.added).collect();
        // Four bytes read leave two, too few for the second file but enough for the third.
        assert_eq!(lines, [Some(2), None, Some(1)]);
    }

    #[test]
    fn numstat_tells_counts_renames_and_binary_files() {
        let files =
            parse_numstat(b"2\t1\ta b.txt\x000\t0\t\x00old.txt\x00new.txt\x00-\t-\tpic.png\x00");
        assert_eq!(files, [
            FileStat {
                path: "a b.txt".to_owned(),
                added: Some(2),
                removed: Some(1),
            },
            FileStat {
                path: "new.txt".to_owned(),
                added: Some(0),
                removed: Some(0),
            },
            FileStat {
                path: "pic.png".to_owned(),
                added: None,
                removed: None,
            },
        ]);
    }
}
