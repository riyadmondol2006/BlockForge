//! Big files in pieces. Many servers give one connection a few MB/s however fast the line is (a modpack's own host,
//! Forge's maven); the same file asked for in 8 MB pieces, several at once, comes in many times faster. A file is only
//! cut up when it is big, its size is known and the server says it takes ranges; the pieces are written straight to
//! their place in the `.part` file and the whole file is checked against its checksum afterwards as usual.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use reqwest::header::{ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_RANGE, RANGE};
use tokio::sync::Semaphore;

use crate::http::client;
use crate::install::{Progress, next_chunk};

/// Files smaller than this come in one piece: a second connection costs more than it brings.
const MIN_SIZE: u64 = if cfg!(test) { 64 * 1024 } else { 16 << 20 };
/// How much one request asks for.
const PIECE: u64 = if cfg!(test) { 16 * 1024 } else { 8 << 20 };
/// How many pieces of one file come at once.
const AT_ONCE: usize = 8;
/// What a piece gathers before it goes to the disk.
const FLUSH: usize = 1 << 20;
/// Tries for one piece; each goes on from where the last one stopped.
const PIECE_TRIES: u32 = 3;

/// Extra connections for pieces, for all files together: a pack with many big mods doesn't open hundreds.
static PIECES: Semaphore = Semaphore::const_new(16);

/// The size of the file `r` (the answer to a plain GET) is sending, when it is worth fetching in pieces: big, sent
/// as it is (not compressed on the way) by a server that takes ranges.
pub(crate) fn worth_it(r: &reqwest::Response) -> Option<u64> {
    let len = r.content_length()?;
    let ranges = r
        .headers()
        .get(ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("bytes"));
    let encoded = r.headers().get(CONTENT_ENCODING).is_some_and(|v| v.as_bytes() != b"identity");
    (r.status() == reqwest::StatusCode::OK && ranges && !encoded && len >= MIN_SIZE).then_some(len)
}

/// Fetch the `len` bytes of the file `first` (the answer to a plain GET) is sending into `part` in pieces, the first
/// of them read from `first` itself, so no request is wasted. `counted` ends as the bytes that came, also on failure, so the caller can take
/// them off the progress again.
pub(crate) async fn download(
    part: &Path,
    len: u64,
    first: reqwest::Response,
    counted: &mut u64,
    progress: &Progress,
) -> Result<()> {
    let got = AtomicU64::new(0);
    let result = download_inner(part, len, first, &got, progress).await;
    *counted = got.load(Ordering::Relaxed);
    result
}

async fn download_inner(
    part: &Path,
    len: u64,
    first: reqwest::Response,
    got: &AtomicU64,
    progress: &Progress,
) -> Result<()> {
    let file = std::fs::File::create(part).with_context(|| format!("writing {}", part.display()))?;
    file.set_len(len).with_context(|| format!("making room for {}", part.display()))?;
    let file = Arc::new(file);
    // the pieces go straight to where the first answer came from, not through its redirects again
    let url = first.url().as_str().to_string();
    let url = url.as_str();
    let mut first = Some(first);
    let mut pieces = futures::stream::iter((0..len.div_ceil(PIECE)).map(|i| {
        let start = i * PIECE;
        let end = (start + PIECE).min(len) - 1;
        piece(url, file.clone(), start, end, if i == 0 { first.take() } else { None }, got, progress)
    }))
    .buffer_unordered(AT_ONCE);
    // the first piece that fails for good stops the others (they are dropped with the stream)
    while let Some(result) = pieces.next().await {
        result?;
    }
    Ok(())
}

/// Bytes `start..=end` of the file, from `first` when it is given and then by asking for what is still missing.
async fn piece(
    url: &str,
    file: Arc<std::fs::File>,
    start: u64,
    end: u64,
    mut first: Option<reqwest::Response>,
    got: &AtomicU64,
    progress: &Progress,
) -> Result<()> {
    let mut at = start;
    let mut last = None;
    for attempt in 0..PIECE_TRIES {
        if attempt > 0 {
            let base = if cfg!(test) { 5 } else { 300 };
            tokio::time::sleep(Duration::from_millis(base * 2u64.pow(attempt - 1))).await;
        }
        if progress.cancelled.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let result = match first.take() {
            Some(r) => fill(r, &file, &mut at, end, got, progress).await,
            None => {
                // the permit is held while the piece comes
                let _permit = PIECES.acquire().await?;
                match ask(url, at, end).await {
                    Ok(r) => fill(r, &file, &mut at, end, got, progress).await,
                    Err(e) => Err(e),
                }
            }
        };
        match result {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("the piece didn't come")))
        .with_context(|| format!("bytes {start}-{end} of {url}"))
}

/// Ask for bytes `at..=end`, and make sure that is what the server is sending.
async fn ask(url: &str, at: u64, end: u64) -> Result<reqwest::Response> {
    let r = client()
        .get(url)
        .header(RANGE, format!("bytes={at}-{end}"))
        // a range of a compressed answer would be a range of the compressed bytes
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .await?;
    if r.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        bail!("the server didn't send a piece (HTTP {})", r.status());
    }
    let from = r
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().strip_prefix("bytes "))
        .and_then(|v| v.split('-').next())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if from != Some(at) || r.headers().get(CONTENT_ENCODING).is_some_and(|v| v.as_bytes() != b"identity") {
        bail!("the server sent another piece than the one asked for");
    }
    Ok(r)
}

/// Write what `r` sends to the file from `at` on, until `end`. `at` moves along with what reached the disk, so a
/// next try asks only for the rest.
async fn fill(
    r: reqwest::Response,
    file: &Arc<std::fs::File>,
    at: &mut u64,
    end: u64,
    got: &AtomicU64,
    progress: &Progress,
) -> Result<()> {
    let mut stream = r.bytes_stream();
    let mut pending: Vec<u8> = Vec::with_capacity(FLUSH);
    let mut error = None;
    while *at + (pending.len() as u64) <= end {
        match next_chunk(&mut stream).await {
            Some(Ok(chunk)) => {
                // the first answer goes on past its piece: only the piece is kept
                let room = (end + 1 - *at - pending.len() as u64) as usize;
                let take = &chunk[..chunk.len().min(room)];
                pending.extend_from_slice(take);
                got.fetch_add(take.len() as u64, Ordering::Relaxed);
                progress.bytes_done.fetch_add(take.len() as u64, Ordering::Relaxed);
                if pending.len() >= FLUSH {
                    flush(file, at, &mut pending).await?;
                }
                if progress.cancelled.load(Ordering::Relaxed) {
                    error = Some(anyhow::anyhow!("cancelled"));
                    break;
                }
            }
            Some(Err(e)) => {
                error = Some(e);
                break;
            }
            None => {
                error = Some(anyhow::anyhow!("the piece came short"));
                break;
            }
        }
    }
    // what came before a failure is good: it is kept, and the next try goes on after it
    flush(file, at, &mut pending).await?;
    match error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

async fn flush(file: &Arc<std::fs::File>, at: &mut u64, pending: &mut Vec<u8>) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let (f, data, offset) = (file.clone(), std::mem::take(pending), *at);
    let n = data.len() as u64;
    let data = tokio::task::spawn_blocking(move || write_at(&f, &data, offset).map(|()| data)).await??;
    *at += n;
    // the buffer goes back for the next round
    *pending = data;
    pending.clear();
    Ok(())
}

#[cfg(unix)]
fn write_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn write_at(file: &std::fs::File, mut buf: &[u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = file.seek_write(buf, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use crate::install::{Job, Progress};
    use crate::packs::dl::Dl;
    use crate::testutil::{Reply, Server, runtime, scratch, sha1};

    /// Bytes that differ along the file, so a piece written to the wrong place shows.
    fn big(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 + i / 251) as u8).collect()
    }

    #[test]
    fn a_big_file_comes_in_pieces_and_whole() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("ranged");
            let body = big(300_000);
            let url = server.route("/pack.zip", Reply::ok(body.clone()).with_ranges());

            // a modpack's file with no size, as the Technic zip is
            let progress = Progress::new();
            crate::packs::dl::download_all(vec![Dl::new(&url, dir.join("pack.zip"))], &progress).await.unwrap();
            assert_eq!(std::fs::read(dir.join("pack.zip")).unwrap(), body);
            assert_eq!(server.hits("/pack.zip"), 300_000usize.div_ceil(16 * 1024), "one request for each piece");
            assert_eq!(progress.bytes_done.load(Ordering::Relaxed), body.len() as u64);
            assert_eq!(progress.bytes_total.load(Ordering::Relaxed), body.len() as u64);

            // a game file, checked by its SHA-1 once all the pieces are in
            let job = Job::new(&url, dir.join("client.jar")).checked(Some(&sha1(&body)), Some(body.len() as i64));
            crate::install::download_all(vec![job], &Progress::new(), false).await.unwrap();
            assert_eq!(std::fs::read(dir.join("client.jar")).unwrap(), body);
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    #[test]
    fn a_piece_that_stalls_goes_on_from_where_it_stopped() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("ranged-stall");
            let body = big(200_000);
            // the first answer (which also carries the first piece) stops after 10 KB and sends nothing more
            let url = server.route("/pack.zip", Reply::stall_first(body.clone(), 10_000).with_ranges());
            let progress = Progress::new();
            let job = Dl::new(&url, dir.join("pack.zip")).hash(Some(&sha1(&body)));
            crate::packs::dl::download_all(vec![job], &progress).await.unwrap();
            assert_eq!(std::fs::read(dir.join("pack.zip")).unwrap(), body);
            assert_eq!(server.hits("/pack.zip"), 200_000usize.div_ceil(16 * 1024) + 1, "only the first piece again");
            assert_eq!(progress.bytes_done.load(Ordering::Relaxed), body.len() as u64, "nothing counted twice");
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    #[test]
    fn small_files_and_servers_without_ranges_come_in_one_go() {
        runtime().block_on(async {
            let server = Server::start().await;
            let dir = scratch("ranged-plain");
            let small = server.route("/small.jar", Reply::ok(big(30_000)).with_ranges());
            let no_ranges = server.route("/big.zip", Reply::ok(big(300_000)));
            let jobs = vec![Dl::new(&small, dir.join("small.jar")), Dl::new(&no_ranges, dir.join("big.zip"))];
            crate::packs::dl::download_all(jobs, &Progress::new()).await.unwrap();
            assert_eq!(server.hits("/small.jar"), 1);
            assert_eq!(server.hits("/big.zip"), 1);
            assert_eq!(std::fs::read(dir.join("big.zip")).unwrap(), big(300_000));
            let _ = std::fs::remove_dir_all(&dir);
        });
    }
}
