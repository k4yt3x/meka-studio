//! A repository's history: the commits a branch is made of, newest first, and what each changed
//! from the commit before it.

use std::path::PathBuf;

use serde::Serialize;
use utoipa::ToSchema;

use super::{
    ChangeKind, Git, GitError, MAX_CHANGES, is_object_id,
    lines::{BIG_FILE, FileStat, parse_numstat},
    run,
};

/// How many commits a page of the history holds.
pub(super) const PAGE: usize = 50;

/// What the history tells of each commit: its id, its parents' ids, its author's name, the time
/// they wrote it, and its message's first line, apart.
const COMMIT_FORMAT: &str = "--format=tformat:%H%x00%P%x00%an%x00%aI%x00%s%x00";

/// A commit, as the history lists it.
#[derive(Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct Commit {
    pub(crate) id: String,
    /// The commits it follows: none for a first commit, two or more for a merge.
    pub(crate) parents: Vec<String>,
    pub(crate) author: String,
    /// When its author wrote it, in RFC 3339.
    pub(crate) date: String,
    /// Its message's first line.
    pub(crate) subject: String,
}

/// A page of a branch's history, newest first.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct History {
    pub(crate) commits: Vec<Commit>,
    /// Whether older commits follow.
    pub(crate) more: bool,
}

/// What a commit changed from the commit before it: its first parent, for a merge.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CommitChanges {
    pub(crate) id: String,
    pub(crate) parents: Vec<String>,
    pub(crate) author: String,
    pub(crate) email: String,
    /// When its author wrote it, in RFC 3339.
    pub(crate) date: String,
    pub(crate) message: String,
    pub(crate) files: Vec<ChangedFile>,
    /// Whether more changed than are told: past 10,000, none are.
    pub(crate) too_many: bool,
}

/// A file a commit changed, with the lines it added and removed: none for a binary file, or one
/// past 10 MiB.
#[derive(Debug, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct ChangedFile {
    /// Where it is in the repository after the commit, its names separated by `/`.
    pub(crate) path: String,
    /// Where it was before, renamed.
    pub(crate) from: Option<String>,
    pub(crate) kind: ChangeKind,
    pub(crate) added: Option<u64>,
    pub(crate) removed: Option<u64>,
}

impl Git {
    /// The commits from `head`, by its full id, back, newest first, past the first `skip`: as
    /// long as `head` is the same, each page follows the one before it.
    pub(crate) async fn history(
        &self,
        repo: PathBuf,
        head: &str,
        skip: usize,
    ) -> Result<History, GitError> {
        if !is_object_id(head) {
            return Err(GitError::BadRequest(format!(
                "{head} is not a commit's full id"
            )));
        }
        self.check(&repo).await?;
        let failed = |detail: String| GitError::Failed(repo.display().to_string(), detail);
        let mut command = self.reading(Some(&repo), &[]);
        command.args(["log", "--no-show-signature", COMMIT_FORMAT]);
        command.arg(format!("--skip={skip}"));
        command.arg(format!("--max-count={}", PAGE + 1));
        command.args([head, "--"]);
        let output = run(command).await.map_err(failed)?;
        if output.capped || !output.status.success() {
            return Err(failed(output.error()));
        }
        let mut commits = parse_commits(&output.stdout);
        let more = commits.len() > PAGE;
        commits.truncate(PAGE);
        Ok(History { commits, more })
    }

    /// What the commit `commit`, by its full id, changed from its first parent, or from nothing
    /// for a first commit.
    pub(crate) async fn commit_changes(
        &self,
        repo: PathBuf,
        commit: &str,
    ) -> Result<CommitChanges, GitError> {
        if !is_object_id(commit) {
            return Err(GitError::BadRequest(format!(
                "{commit} is not a commit's full id"
            )));
        }
        self.check(&repo).await?;
        let failed = |detail: String| GitError::Failed(repo.display().to_string(), detail);
        let mut command = self.reading(Some(&repo), &[]);
        command.args([
            "show",
            "-s",
            "--no-show-signature",
            "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%B",
            commit,
            "--",
        ]);
        let shown = run(command).await.map_err(failed)?;
        if shown.capped || !shown.status.success() {
            return Err(failed(shown.error()));
        }
        let text = String::from_utf8_lossy(&shown.stdout);
        let mut fields = text.splitn(6, '\0');
        let (Some(id), Some(parents), Some(author), Some(email), Some(date), Some(message)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(failed(format!("git told of {commit} as {text}")));
        };
        let parents: Vec<String> = parents.split_whitespace().map(str::to_owned).collect();
        let config = [("core.bigFileThreshold".to_owned(), BIG_FILE.to_owned())];
        let mut command = self.reading(Some(&repo), &config);
        command.args([
            "diff-tree",
            "-r",
            "-z",
            "-M",
            "--raw",
            "--numstat",
            "--no-commit-id",
            "--no-textconv",
            "--no-ext-diff",
        ]);
        match parents.first() {
            Some(parent) => command.args([parent.as_str(), id]),
            None => command.args(["--root", id]),
        };
        let changed = run(command).await.map_err(failed)?;
        if !changed.capped && !changed.status.success() {
            return Err(failed(changed.error()));
        }
        let files = parse_changes(&changed.stdout);
        let too_many = changed.capped || files.len() > MAX_CHANGES;
        Ok(CommitChanges {
            id: id.to_owned(),
            parents,
            author: author.to_owned(),
            email: email.to_owned(),
            date: date.to_owned(),
            message: message.trim_end().to_owned(),
            files: if too_many { Vec::new() } else { files },
            too_many,
        })
    }
}

/// The commits [`COMMIT_FORMAT`] tells of in `output`.
fn parse_commits(output: &[u8]) -> Vec<Commit> {
    let text = String::from_utf8_lossy(output);
    let fields: Vec<&str> = text.split('\0').collect();
    fields
        .as_chunks::<5>()
        .0
        .iter()
        .map(|[id, parents, author, date, subject]| Commit {
            // Each commit after the first starts on a line of its own.
            id: id.trim_start_matches('\n').to_owned(),
            parents: parents.split_whitespace().map(str::to_owned).collect(),
            author: (*author).to_owned(),
            date: (*date).to_owned(),
            subject: (*subject).to_owned(),
        })
        .collect()
}

/// The files `git diff-tree -z --raw --numstat` tells of in `output`: first each as it changed,
/// its modes, ids and kind, then a NUL and its path, or for a rename or copy where it was and where
/// it is; then the lines each added and removed, as `--numstat` tells of them.
fn parse_changes(output: &[u8]) -> Vec<ChangedFile> {
    let mut changes = Vec::new();
    let mut records = output.split(|byte| *byte == 0).peekable();
    while let Some(record) = records.peek() {
        let Some(raw) = record.strip_prefix(b":") else {
            break;
        };
        let kind = raw
            .split(|byte| *byte == b' ')
            .next_back()
            .and_then(|status| status.first().copied());
        records.next();
        let mut path = || {
            records
                .next()
                .map(|path| String::from_utf8_lossy(path).into_owned())
                .unwrap_or_default()
        };
        let (from, path) = if matches!(kind, Some(b'R' | b'C')) {
            let from = path();
            (Some(from), path())
        } else {
            (None, path())
        };
        changes.push(ChangedFile {
            path,
            // A copy keeps the file it came from: only the new one is told of, as added.
            from: from.filter(|_| kind == Some(b'R')),
            kind: match kind {
                Some(b'A' | b'C') => ChangeKind::Added,
                Some(b'D') => ChangeKind::Deleted,
                Some(b'R') => ChangeKind::Renamed,
                _ => ChangeKind::Modified,
            },
            added: None,
            removed: None,
        });
    }
    let rest: Vec<u8> = records.collect::<Vec<_>>().join(&0);
    for FileStat {
        path,
        added,
        removed,
    } in parse_numstat(&rest)
    {
        if let Some(change) = changes.iter_mut().find(|change| change.path == path) {
            change.added = added;
            change.removed = removed;
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_read_with_their_parents_and_subjects() {
        let commits = parse_commits(
            b"aaa\0bbb ccc\0Ann\x002026-11-18T10:00:00+00:00\0merge: two\0\nbbb\0\0Bo\x002026-11-17T10:00:00+00:00\0first\0\n",
        );
        assert_eq!(commits, [
            Commit {
                id: "aaa".to_owned(),
                parents: vec!["bbb".to_owned(), "ccc".to_owned()],
                author: "Ann".to_owned(),
                date: "2026-11-18T10:00:00+00:00".to_owned(),
                subject: "merge: two".to_owned(),
            },
            Commit {
                id: "bbb".to_owned(),
                parents: Vec::new(),
                author: "Bo".to_owned(),
                date: "2026-11-17T10:00:00+00:00".to_owned(),
                subject: "first".to_owned(),
            },
        ]);
    }

    #[test]
    fn a_commit_tells_each_file_it_changed_with_its_lines() {
        let changes = parse_changes(
            concat!(
                ":100644 100644 aaa bbb M\0a.txt\0",
                ":100644 100644 ccc ccc R100\0old.txt\0new.txt\0",
                ":000000 100644 000 ddd A\0pic.png\0",
                ":100644 000000 eee 000 D\0gone.txt\0",
                "2\t1\ta.txt\0",
                "0\t0\t\0old.txt\0new.txt\0",
                "-\t-\tpic.png\0",
                "0\t3\tgone.txt\0",
            )
            .as_bytes(),
        );
        let file = |path: &str, from: Option<&str>, kind, added, removed| ChangedFile {
            path: path.to_owned(),
            from: from.map(str::to_owned),
            kind,
            added,
            removed,
        };
        assert_eq!(changes, [
            file("a.txt", None, ChangeKind::Modified, Some(2), Some(1)),
            file(
                "new.txt",
                Some("old.txt"),
                ChangeKind::Renamed,
                Some(0),
                Some(0)
            ),
            file("pic.png", None, ChangeKind::Added, None, None),
            file("gone.txt", None, ChangeKind::Deleted, Some(0), Some(3)),
        ]);
    }
}
