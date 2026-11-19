//! Reading an HTTP answer's body, bounded: the update check's from GitHub, and Meka's own.

use anyhow::bail;
use futures_util::StreamExt;

/// The body of `response`, refused once it is larger than `max` bytes.
pub(crate) async fn read_at_most(
    response: reqwest::Response,
    max: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk?);
        if body.len() > max {
            bail!("the answer is larger than {max} bytes");
        }
    }
    Ok(body)
}
