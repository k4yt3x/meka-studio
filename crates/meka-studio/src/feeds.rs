//! One event stream for each page, carrying every feed it follows: Meka's session feeds, its
//! server feed, and changes to folders. Over HTTP/1.1, which the desktop app always speaks, a
//! browser holds about six connections to a server, so a page holding one for each feed, as for
//! an agent's many sub-agents, would leave none for its other requests.
//!
//! A page opens its stream, which names itself first, then asks for each feed under a key of its
//! own and lets it go by that key. Every line the stream carries is `data: <key> <line>`, a line of
//! that feed's own event stream as it came, so the page hands each feed on as a stream of its own.
//! The key `!` carries what the stream says of the feeds: `opened <stream>`, `started <key>`,
//! `ended <key>`, and `failed <key> <status> <retry-after seconds, or -> <problem>`.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::http::{HeaderValue, header};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::{sync::mpsc, task::AbortHandle};
use utoipa::ToSchema;

use crate::{
    api::files::{FolderEvent, MAX_WATCHED, watch_folders},
    app::App,
    error::ApiError,
    proxy::{MAX_BATCH_BYTES, compress, forward_bytes, rewrite},
    sse::{Gzip, Rewriter},
};

/// The most feeds one stream follows at once.
const MAX_FOLLOWED: usize = 64;

/// The most streams one sign-in holds open at once: a page each, and a browser's tabs share one
/// sign-in.
const MAX_STREAMS: usize = 32;

/// How often an idle stream sends a comment, so proxies keep it open.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The most of Meka's refusal read, to say why a feed could not be followed.
const MAX_PROBLEM_BYTES: usize = 64 * 1024;

/// The longest key a feed is followed under.
const MAX_KEY: usize = 64;

/// The longest line of a feed carried; the rest of a longer one is left out, as it would have to
/// be held whole to go on under its key.
const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

/// What a page asks its stream to follow.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum Follow {
    /// Meka's feed at `path`, its server feed `/v1/feed` or a session's `/v1/sessions/{id}/feed`,
    /// from `cursor`, as `Last-Event-ID` resumes it.
    Feed {
        path: String,
        #[serde(default)]
        cursor: Option<String>,
        /// Whether the page answers the session's prompts, as Meka's `attend` asks.
        #[serde(default)]
        attend: Option<bool>,
        /// Whether the page is in front, alerting of finished turns itself, so its device is sent
        /// no push notifications of them; for the server feed.
        #[serde(default)]
        focused: bool,
    },
    /// Changes to folders, at most 64, as `change`, `gone` and `unwatched` events whose data is
    /// the folder as asked for. What changed while they were not followed is not told, so they are
    /// listed again as following starts.
    Folders { paths: Vec<String> },
}

/// Every page's stream, by its name.
#[derive(Debug, Default)]
pub(crate) struct Feeds {
    streams: Mutex<HashMap<String, Arc<Stream>>>,
}

/// A page's stream: whose it is, and what it follows.
#[derive(Debug)]
pub(crate) struct Stream {
    device: String,
    /// Whether the device saves data, leaving commands' streamed output out.
    lean: bool,
    out: mpsc::Sender<Out>,
    follows: Mutex<HashMap<String, (u64, AbortHandle)>>,
    next: AtomicU64,
}

impl Feeds {
    /// Opens a stream for the sign-in `device`: its name, itself, and what it is to carry.
    pub(crate) fn open(
        &self,
        device: &str,
        lean: bool,
    ) -> Result<(String, Arc<Stream>, mpsc::Receiver<Out>), ApiError> {
        let mut streams = self.streams.lock().unwrap_or_else(PoisonError::into_inner);
        if streams
            .values()
            .filter(|stream| stream.device == device)
            .count()
            >= MAX_STREAMS
        {
            return Err(ApiError::too_many(5));
        }
        let name = uuid::Uuid::new_v4().to_string();
        let (out, receiver) = mpsc::channel(64);
        let stream = Arc::new(Stream {
            device: device.to_owned(),
            lean,
            out,
            follows: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
        });
        streams.insert(name.clone(), Arc::clone(&stream));
        drop(streams);
        Ok((name, stream, receiver))
    }

    /// The stream `name` of the sign-in `device`; another's is no more found than a missing one.
    pub(crate) fn find(&self, name: &str, device: &str) -> Result<Arc<Stream>, ApiError> {
        self.streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .filter(|stream| stream.device == device)
            .cloned()
            .ok_or_else(|| ApiError::not_found("no such stream is open"))
    }

    /// Closes the stream `name`, letting go of all it follows.
    pub(crate) fn close(&self, name: &str) {
        let removed = self
            .streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
        if let Some(stream) = removed {
            for (_, (_, task)) in stream.follows_map().drain() {
                task.abort();
            }
        }
    }
}

impl Stream {
    fn follows_map(&self) -> std::sync::MutexGuard<'_, HashMap<String, (u64, AbortHandle)>> {
        self.follows.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Follows `follow` under `key`, in place of what that key followed.
    pub(crate) fn follow(
        self: &Arc<Self>,
        app: &App,
        key: &str,
        follow: Follow,
    ) -> Result<(), ApiError> {
        if key.is_empty()
            || key.len() > MAX_KEY
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
        {
            return Err(ApiError::bad_request(format!(
                "a key is 1 to {MAX_KEY} letters, digits, dots, colons, dashes or underscores"
            )));
        }
        match &follow {
            Follow::Feed { path, .. } if session_of(path).is_none() && path != "/v1/feed" => {
                return Err(ApiError::bad_request(
                    "a feed is /v1/feed or /v1/sessions/{id}/feed",
                ));
            }
            Follow::Feed {
                cursor: Some(cursor),
                ..
            } if HeaderValue::from_str(cursor).is_err() => {
                return Err(ApiError::bad_request(
                    "a cursor is the id of an event the feed sent",
                ));
            }
            Follow::Folders { paths } if paths.len() > MAX_WATCHED => {
                return Err(ApiError::bad_request(format!(
                    "follow at most {MAX_WATCHED} folders at once"
                )));
            }
            _ => {}
        }
        let mut follows = self.follows_map();
        if !follows.contains_key(key) && follows.len() >= MAX_FOLLOWED {
            return Err(ApiError::bad_request(format!(
                "a stream follows at most {MAX_FOLLOWED} feeds at once"
            )));
        }
        let following = Following {
            stream: Arc::clone(self),
            key: key.to_owned(),
            generation: self.next.fetch_add(1, Ordering::Relaxed),
        };
        let generation = following.generation;
        let app = app.clone();
        let task = tokio::spawn(async move {
            match follow {
                Follow::Feed {
                    path,
                    cursor,
                    attend,
                    focused,
                } => relay_feed(&app, &following, &path, cursor, attend, focused).await,
                Follow::Folders { paths } => relay_folders(&app, &following, paths).await,
            }
            let mut follows = following.stream.follows_map();
            if following.is_current(&follows) {
                follows.remove(&following.key);
            }
        });
        let replaced = follows.insert(key.to_owned(), (generation, task.abort_handle()));
        drop(follows);
        if let Some((_, replaced)) = replaced {
            replaced.abort();
        }
        Ok(())
    }

    /// Lets go of what `key` follows.
    pub(crate) fn unfollow(&self, key: &str) {
        if let Some((_, task)) = self.follows_map().remove(key) {
            task.abort();
        }
    }
}

/// What goes out on a stream: lines, and whether they go at once rather than with the next batch.
#[derive(Debug)]
pub(crate) struct Out {
    text: String,
    now: bool,
}

/// A feed followed on a stream, under its key, until the page lets it go or follows another there.
struct Following {
    stream: Arc<Stream>,
    key: String,
    generation: u64,
}

impl Following {
    /// Whether the key still follows this, in `follows`.
    fn is_current(&self, follows: &HashMap<String, (u64, AbortHandle)>) -> bool {
        follows
            .get(&self.key)
            .is_some_and(|(at, _)| *at == self.generation)
    }

    /// Sends `out`; whether the page is still there and still follows this.
    async fn send(&self, out: Out) -> bool {
        let Ok(permit) = self.stream.out.reserve().await else {
            return false;
        };
        // Looked at as it is sent, so nothing of a feed let go of comes after the page let it go
        // or after what followed it under its key started, however late its task stops.
        let follows = self.stream.follows_map();
        if !self.is_current(&follows) {
            return false;
        }
        permit.send(out);
        true
    }

    /// Says `what` of the feed, at once even for a device saving data, as the page waits on it to
    /// load what the feed is of; whether to go on.
    async fn say(&self, what: &str) -> bool {
        self.send(Out {
            text: format!("data: ! {what}\n\n"),
            now: true,
        })
        .await
    }

    /// Carries `lines` of the feed; whether to go on.
    async fn carry(&self, lines: Vec<String>) -> bool {
        if lines.is_empty() {
            return true;
        }
        let mut text = String::new();
        for line in lines {
            text.push_str("data: ");
            text.push_str(&self.key);
            text.push(' ');
            text.push_str(&line);
            text.push('\n');
        }
        text.push('\n');
        self.send(Out { text, now: false }).await
    }
}

/// The session whose feed `path` is, if it is a session's feed.
fn session_of(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/v1/sessions/")?.strip_suffix("/feed")?;
    (!id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    .then_some(id)
}

/// Says that the feed could not be followed, for `error`.
async fn refuse(following: &Following, error: &ApiError) {
    let problem = error.problem();
    let json = serde_json::to_string(&problem).unwrap_or_default();
    following
        .say(&format!(
            "failed {} {} - {json}",
            following.key, problem.status
        ))
        .await;
}

/// Follows Meka's feed at `path` until it ends, the page lets it go or leaves.
async fn relay_feed(
    app: &App,
    following: &Following,
    path: &str,
    cursor: Option<String>,
    attend: Option<bool>,
    focused: bool,
) {
    // A session's feed marks its device as watching the session, so push notifications skip what
    // someone is looking at; a server feed in front, as alerting itself.
    let stream = &following.stream;
    let key = &following.key;
    let _guard = session_of(path)
        .map(|session| app.watching.watch(&stream.device, session))
        .or_else(|| focused.then(|| app.watching.focus(&stream.device)));
    let mut url = app.meka.base().clone();
    url.set_path(path);
    if let Some(attend) = attend {
        url.query_pairs_mut()
            .append_pair("attend", if attend { "true" } else { "false" });
    }
    let mut request = app
        .meka
        .client()
        .get(url)
        .bearer_auth(app.meka.token())
        .header(header::ACCEPT, "text/event-stream");
    if let Some(cursor) = cursor {
        request = request.header("Last-Event-ID", cursor);
    }
    let response = match request.send().await {
        Ok(response) => {
            app.meka.answered();
            response
        }
        Err(error) => {
            let detail = if error.is_connect() || error.is_timeout() {
                app.meka.lost(&error.to_string());
                format!("Meka Studio could not reach Meka at {}.", app.meka.base())
            } else {
                format!("The request to Meka failed: {error}")
            };
            refuse(following, &ApiError::meka_unavailable(detail)).await;
            return;
        }
    };
    // Meka refusing its token is Meka Studio's trouble, not a sign that the browser is signed out.
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        app.meka.lost("Meka refused Meka Studio's token");
        let error = ApiError::meka_unavailable(
            "Meka refused Meka Studio's token; check meka.token or meka.token_file in Meka Studio's config.",
        );
        refuse(following, &error).await;
        return;
    }
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let retry = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map_or_else(|| "-".to_owned(), |seconds| seconds.to_string());
        let mut body = Vec::new();
        let mut chunks = response.bytes_stream();
        while let Some(Ok(chunk)) = chunks.next().await {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_PROBLEM_BYTES {
                body.clear();
                break;
            }
        }
        // On one line, as the stream carries it.
        let problem = serde_json::from_slice::<serde_json::Value>(&body)
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "{}".to_owned());
        following
            .say(&format!("failed {key} {status} {retry} {problem}"))
            .await;
        return;
    }
    if !following.say(&format!("started {key}")).await {
        return;
    }
    let mut upstream = response.bytes_stream();
    let mut rewriter = Rewriter::new(stream.lean);
    let mut lines = Lines::default();
    loop {
        // The page leaving ends the relay at once, rather than at Meka's next write.
        let chunk = tokio::select! {
            chunk = upstream.next() => chunk,
            () = stream.out.closed() => return,
        };
        match chunk {
            Some(Ok(chunk)) => {
                // Rewriting failing ends the feed, which the page follows again from where it was.
                let Some(out) = rewrite(&mut rewriter, chunk).await else {
                    break;
                };
                if !following.carry(lines.push(&out)).await {
                    return;
                }
            }
            Some(Err(error)) => {
                tracing::debug!("Meka's event stream ended: {error}");
                break;
            }
            None => break,
        }
    }
    let mut rest = lines.push(&rewriter.finish());
    rest.extend(lines.finish());
    if following.carry(rest).await {
        following.say(&format!("ended {key}")).await;
    }
}

/// Follows changes to the folders `paths` until the page lets them go or leaves.
async fn relay_folders(app: &App, following: &Following, paths: Vec<String>) {
    // Watched before the page hears that following started, as it lists the folders again then.
    let folders = watch_folders(app, paths).await;
    if !following.say(&format!("started {}", following.key)).await {
        return;
    }
    let (sender, mut receiver) = mpsc::channel::<FolderEvent>(16);
    let tell = async {
        while let Some(event) = receiver.recv().await {
            let mut lines = vec![format!("event: {}", event.kind)];
            lines.extend(event.path.split('\n').map(|line| format!("data: {line}")));
            lines.push(String::new());
            if !following.carry(lines).await {
                return;
            }
        }
    };
    tokio::select! {
        () = folders.tell(&sender) => {}
        () = tell => {}
        () = following.stream.out.closed() => {}
    }
}

/// Splits what a feed sends into its lines as they complete.
#[derive(Debug)]
struct Lines {
    partial: Vec<u8>,
    /// Whether the rest of the line `partial` would hold is left out, for its length.
    skipping: bool,
    /// The longest line carried.
    limit: usize,
}

impl Default for Lines {
    fn default() -> Self {
        Self {
            partial: Vec::new(),
            skipping: false,
            limit: MAX_LINE_BYTES,
        }
    }
}

impl Lines {
    /// The lines `bytes` completes, without their ends.
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let mut pieces = bytes.split(|byte| *byte == b'\n');
        if let Some(first) = pieces.next() {
            self.hold(first);
        }
        // Each piece after the first follows a line's end.
        for piece in pieces {
            let line = std::mem::take(&mut self.partial);
            if !std::mem::take(&mut self.skipping) {
                lines.push(text_of(line));
            }
            self.hold(piece);
        }
        lines
    }

    /// Holds `piece` of a line until it ends, unless the line is too long to carry.
    fn hold(&mut self, piece: &[u8]) {
        if self.skipping {
            return;
        }
        if self.partial.len() + piece.len() > self.limit {
            tracing::warn!("left out a line of a feed longer than {} bytes", self.limit);
            self.partial = Vec::new();
            self.skipping = true;
            return;
        }
        self.partial.extend_from_slice(piece);
    }

    /// The last line, if the feed ended without ending it.
    fn finish(&mut self) -> Option<String> {
        let line = std::mem::take(&mut self.partial);
        (!std::mem::take(&mut self.skipping) && !line.is_empty()).then(|| text_of(line))
    }
}

/// A line's text, without the carriage return that may end it.
fn text_of(mut line: Vec<u8>) -> String {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

/// Writes a page's stream from `receiver` to `sender` until either end stops or `ended` comes:
/// compressed with `gzip`, and gathered for `batch` at a time for a device saving data.
pub(crate) async fn write(
    name: &str,
    mut receiver: mpsc::Receiver<Out>,
    sender: &mpsc::Sender<Result<Bytes, std::io::Error>>,
    gzip: bool,
    batch: Option<Duration>,
    ended: impl Future<Output = ()>,
) {
    tokio::pin!(ended);
    let mut encoder = gzip.then(Gzip::new);
    let mut pending = format!("data: ! opened {name}\n\n").into_bytes();
    let mut now = true;
    let mut ticker = batch.map(tokio::time::interval);
    let mut idle = tokio::time::interval(KEEP_ALIVE);
    idle.reset();
    loop {
        if !pending.is_empty() && (batch.is_none() || now || pending.len() >= MAX_BATCH_BYTES) {
            let out = std::mem::take(&mut pending);
            if !forward_bytes(sender, compress(&mut encoder, out)).await {
                return;
            }
            idle.reset();
        }
        now = false;
        let tick = async {
            match &mut ticker {
                Some(ticker) => ticker.tick().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            out = receiver.recv() => match out {
                Some(out) => {
                    pending.extend_from_slice(out.text.as_bytes());
                    now = out.now;
                }
                None => return,
            },
            _ = tick => {
                let out = std::mem::take(&mut pending);
                if !forward_bytes(sender, compress(&mut encoder, out)).await {
                    return;
                }
            }
            _ = idle.tick() => pending.extend_from_slice(b": \n\n"),
            () = sender.closed() => return,
            // What is held goes, and the compressed stream ends whole, so the page sees the
            // stream end rather than break.
            () = &mut ended => {
                let out = std::mem::take(&mut pending);
                if !forward_bytes(sender, compress(&mut encoder, out)).await {
                    return;
                }
                if let Some(encoder) = encoder.take() {
                    match encoder.finish() {
                        Ok(end) => {
                            if sender.send(Ok(Bytes::from(end))).await.is_err() {
                                tracing::debug!("a page left as its stream ended");
                            }
                        }
                        Err(error) => tracing::warn!("could not end a compressed stream: {error}"),
                    }
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_come_whole_however_the_feed_is_cut() {
        let mut lines = Lines::default();
        assert_eq!(lines.push(b"id: 1\r\nevent: a"), ["id: 1"]);
        assert_eq!(lines.push(b"\ndata: {}\n\n"), ["event: a", "data: {}", ""]);
        assert_eq!(lines.push(b"data: x"), Vec::<String>::new());
        assert_eq!(lines.finish().as_deref(), Some("data: x"));
        assert_eq!(lines.finish(), None);
    }

    #[test]
    fn a_line_too_long_to_hold_is_left_out_and_the_rest_carried() {
        let mut lines = Lines {
            limit: 8,
            ..Lines::default()
        };
        assert_eq!(lines.push(b"id: 1\ndata: 0123"), ["id: 1"]);
        assert_eq!(lines.push(b"4567"), Vec::<String>::new());
        assert_eq!(lines.push(b"89\n\nid: 2\n"), ["", "id: 2"]);
        assert_eq!(lines.push(b"data: 0123456789\nok\n"), ["ok"]);
        assert_eq!(lines.push(b"data: 0123456789"), Vec::<String>::new());
        assert_eq!(lines.finish(), None);
    }

    #[test]
    fn only_meka_s_feeds_are_followed() {
        assert_eq!(session_of("/v1/sessions/0a-1b/feed"), Some("0a-1b"));
        for path in [
            "/v1/sessions//feed",
            "/v1/sessions/a/b/feed",
            "/v1/sessions/../feed",
            "/v1/sessions/a%2Fb/feed",
            "/v1/sessions/a/turns",
        ] {
            assert_eq!(session_of(path), None, "{path}");
        }
    }
}
