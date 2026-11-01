//! BlockForge addition (not upstream Xodus): checks an XVD against its own hash tree, starting from a header that the
//! caller obtained over an authenticated channel.
//!
//! What the format commits to, as confirmed on Minecraft 1.26.6005.0's Store package (see PATCHES.md):
//!
//! - The header (the first three pages) holds `top_hash_block_hash`: the SHA-256 of the one page of the topmost
//!   hash-tree level.
//! - The levels are stored top first. Page `j` of a lower level is covered by entry `j` of the level above it: the
//!   first 24 bytes of the page's SHA-256. An entry is 24 bytes, 170 to a page, the last 16 bytes of a page unused.
//! - Entry `i` of level 0 covers hashed page `i`, the hashed pages being everything from the user data to the end of
//!   the drive (user data, XVC info with its region table, dynamic header, drive). The entry is the page's SHA-256 as
//!   stored: over the ciphertext in an encrypted region, whose entry keeps only 20 bytes of it and ends in the XTS
//!   data unit; 24 bytes of it elsewhere. Leaves are compared on those first 20 bytes.
//!
//! The header itself is not checked here (its RSA signature needs Microsoft's public key, which is not published):
//! [`Plan::new`] takes it as trusted, so the caller must have it from somewhere an attacker can't change.
//! [`VerifyingReader`] then refuses a stream whose header differs from it.

use crate::layout::PAGE_SIZE;
use crate::models::xvd::{XvdHeader, XvdType};

use msixvc_common::parse::BinaryTryParse;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, ReadBuf};

use std::io::{self, SeekFrom};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};

/// The header's length: three pages, the first 0x200 bytes of them its signature.
pub const HEADER_LEN: usize = 3 * PAGE_SIZE;
/// Bytes of a hash-tree entry, and the prefix of SHA-256 an upper-level entry holds.
const NODE: usize = 24;
/// Bytes of SHA-256 a level-0 entry holds for every page (an encrypted page's entry ends in its XTS data unit).
pub const LEAF: usize = 20;
const PER_PAGE: u32 = (PAGE_SIZE / NODE) as u32;

/// Why a package was refused. Every variant is a refusal: nothing it describes may be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntegrityError {
    #[error("the package's header differs from the copy fetched over HTTPS")]
    HeaderMismatch,
    #[error("hash-tree page {page} does not match its hash")]
    TreeMismatch { page: u32 },
    #[error("the package's page at offset {offset:#x} does not match its hash")]
    PageMismatch { offset: u64 },
    #[error("the package's page at offset {offset:#x} is not covered by its hash tree")]
    Unhashed { offset: u64 },
    #[error("the package's hash tree has not been verified")]
    NoTree,
    #[error("the package can't be verified: {0}")]
    Unsupported(&'static str),
    #[error("the package's header is invalid: {0}")]
    Header(String),
}

impl From<IntegrityError> for io::Error {
    fn from(e: IntegrityError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, e)
    }
}

/// The [`IntegrityError`] somewhere in `error`'s chain, also inside an [`io::Error`], if any: lets a caller tell a
/// verification refusal from an ordinary I/O or format failure.
pub fn find<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a IntegrityError> {
    let mut current = Some(error);
    while let Some(e) = current {
        if let Some(found) = e.downcast_ref::<IntegrityError>() {
            return Some(found);
        }
        if let Some(found) = e.downcast_ref::<io::Error>().and_then(|io| io.get_ref()).and_then(|inner| {
            let inner: &(dyn std::error::Error + 'static) = inner;
            find(inner)
        }) {
            return Some(found);
        }
        current = e.source();
    }
    None
}

fn sha256(page: &[u8]) -> [u8; 32] {
    Sha256::digest(page).into()
}

/// Where the hashed parts of a package lie, worked out from its trusted header.
#[derive(Clone)]
pub struct Plan {
    header: Box<[u8; HEADER_LEN]>,
    parsed: XvdHeader,
    /// Byte offset of the hash tree.
    tree_start: u64,
    /// The non-empty levels, top first: (first page relative to the tree, pages).
    levels: Vec<(u32, u32)>,
    tree_pages: u32,
    /// Byte offset of the first hashed page (the user data).
    hashed_start: u64,
    hashed_pages: u32,
    /// Byte offset of the drive: the hashed pages before it are metadata.
    drive_start: u64,
    len: u64,
}

impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plan")
            .field("tree_start", &self.tree_start)
            .field("levels", &self.levels)
            .field("hashed_start", &self.hashed_start)
            .field("hashed_pages", &self.hashed_pages)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

impl Plan {
    /// The layout `header` describes. Refuses a package the hash tree can't vouch for: integrity disabled, dynamic
    /// or resilient layouts (their trees are laid out differently and no Store package uses them).
    pub fn new(header: &[u8; HEADER_LEN]) -> Result<Self, IntegrityError> {
        let first: &[u8; PAGE_SIZE] = header[..PAGE_SIZE].try_into().expect("the header holds a whole page");
        let parsed = XvdHeader::try_from_array(first.into()).map_err(|e| IntegrityError::Header(e.to_string()))?;
        if !parsed.volume_flags.is_data_integrity_enabled() {
            return Err(IntegrityError::Unsupported("its hash tree is disabled"));
        }
        if parsed.volume_flags.is_resiliency_enabled() {
            return Err(IntegrityError::Unsupported("it uses a resilient hash tree"));
        }
        if parsed.xvd_type != XvdType::Fixed {
            return Err(IntegrityError::Unsupported("it is a dynamic XVD"));
        }
        if parsed.number_of_hashed_pages().0 == 0 {
            return Err(IntegrityError::Header("no hashed pages".into()));
        }
        let layout = parsed.layout();
        let tree = layout.hash_tree_layout;
        let levels: Vec<(u32, u32)> = [tree.level3, tree.level2, tree.level1, tree.level0]
            .iter()
            .filter(|l| l.num_pages().0 > 0)
            .map(|l| (l.page_range.start.0, l.num_pages().0))
            .collect();
        // the top level is one page, the one the header's hash covers
        if levels.first().map(|l| l.1) != Some(1) {
            return Err(IntegrityError::Header("the top hash-tree level is not one page".into()));
        }
        let tree_start = layout.hash_tree.start.to_bytes().0;
        let hashed_start = layout.user_data.start.to_bytes().0;
        let drive_start = layout.drive_data.start.to_bytes().0;
        let len = drive_start
            .checked_add(layout.drive_data.len.to_page_count().to_bytes().0)
            .ok_or_else(|| IntegrityError::Header("size overflow".into()))?;
        if hashed_start != tree_start + tree.pages().to_bytes().0 || (tree_start as usize) < HEADER_LEN {
            return Err(IntegrityError::Header("unexpected layout".into()));
        }
        let hashed_pages = parsed.number_of_hashed_pages().0;
        Ok(Self {
            header: Box::new(*header),
            parsed,
            tree_start,
            levels,
            tree_pages: tree.pages().0,
            hashed_start,
            hashed_pages,
            drive_start,
            len,
        })
    }

    /// The package's length in bytes, as its header lays it out.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn header(&self) -> &XvdHeader {
        &self.parsed
    }

    fn top_hash(&self) -> [u8; 32] {
        self.parsed.top_hash_block_hash
    }

    fn tree_end(&self) -> u64 {
        self.tree_start + self.tree_pages as u64 * PAGE_SIZE as u64
    }
}

/// The hash tree of a package, every page of it checked against the header's top hash.
pub struct HashTree {
    plan: Plan,
    tree: Vec<u8>,
}

impl std::fmt::Debug for HashTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HashTree").field("plan", &self.plan).finish_non_exhaustive()
    }
}

impl HashTree {
    fn entry(&self, level: usize, index: u32) -> &[u8] {
        entry(&self.tree, self.plan.levels[level], index)
    }

    /// Checks a page of the package, as stored (ciphertext where it is encrypted), that starts at byte `offset`.
    pub fn verify_page(&self, offset: u64, page: &[u8; PAGE_SIZE]) -> Result<(), IntegrityError> {
        let leaf = self.leaf(offset)?;
        if sha256(page)[..LEAF] != leaf[..] {
            return Err(IntegrityError::PageMismatch { offset });
        }
        Ok(())
    }

    /// The 20 bytes of SHA-256 the tree holds for the page that starts at byte `offset`.
    pub fn leaf(&self, offset: u64) -> Result<[u8; LEAF], IntegrityError> {
        let index = offset
            .checked_sub(self.plan.hashed_start)
            .filter(|rel| rel % PAGE_SIZE as u64 == 0)
            .map(|rel| rel / PAGE_SIZE as u64)
            .filter(|&i| i < self.plan.hashed_pages as u64)
            .ok_or(IntegrityError::Unhashed { offset })?;
        let entry = self.entry(self.plan.levels.len() - 1, index as u32);
        Ok(entry[..LEAF].try_into().expect("entries are 24 bytes"))
    }

    /// The trusted header the tree was checked against.
    pub fn header_bytes(&self) -> &[u8; HEADER_LEN] {
        &self.plan.header
    }

    pub fn header(&self) -> &XvdHeader {
        &self.plan.parsed
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Reads a package's header and hash tree from `reader` (a whole package, or the metadata prefix BlockForge
    /// keeps of one), checks the tree against that header, and then every metadata page (user data, XVC info and its
    /// region table, dynamic header) against the tree. The header is whatever `reader` holds: the caller compares
    /// [`Self::header_bytes`] with the one it trusts.
    pub async fn load<R: AsyncRead + AsyncSeek + Unpin>(reader: &mut R) -> Result<Arc<HashTree>, io::Error> {
        reader.seek(SeekFrom::Start(0)).await?;
        let mut header = Box::new([0u8; HEADER_LEN]);
        reader.read_exact(&mut header[..]).await?;
        let plan = Plan::new(&header)?;
        let mut builder = TreeBuilder::new(plan);
        reader.seek(SeekFrom::Start(builder.plan.tree_start)).await?;
        let mut page = Box::new([0u8; PAGE_SIZE]);
        while !builder.done() {
            reader.read_exact(&mut page[..]).await?;
            builder.push(&page)?;
        }
        let tree = builder.finish();
        let mut offset = tree.plan.hashed_start;
        while offset < tree.plan.drive_start {
            reader.read_exact(&mut page[..]).await?;
            tree.verify_page(offset, &page)?;
            offset += PAGE_SIZE as u64;
        }
        Ok(Arc::new(tree))
    }
}

fn entry(tree: &[u8], (start, _): (u32, u32), index: u32) -> &[u8] {
    let page = (start + index / PER_PAGE) as usize;
    let at = page * PAGE_SIZE + (index % PER_PAGE) as usize * NODE;
    &tree[at..at + NODE]
}

/// Takes the hash-tree pages in the order they are stored and checks each against what is already known: the top
/// page against the header, every other one against its entry in the level above (stored before it).
struct TreeBuilder {
    plan: Plan,
    tree: Vec<u8>,
    next: u32,
}

impl TreeBuilder {
    fn new(plan: Plan) -> Self {
        let tree = Vec::with_capacity(plan.tree_pages as usize * PAGE_SIZE);
        Self { plan, tree, next: 0 }
    }

    fn done(&self) -> bool {
        self.next == self.plan.tree_pages
    }

    fn push(&mut self, page: &[u8; PAGE_SIZE]) -> Result<(), IntegrityError> {
        let index = self.next;
        let level = self
            .plan
            .levels
            .iter()
            .position(|&(start, pages)| index >= start && index < start + pages)
            .ok_or(IntegrityError::TreeMismatch { page: index })?;
        let digest = sha256(page);
        let good = if level == 0 {
            digest == self.plan.top_hash()
        } else {
            let position = index - self.plan.levels[level].0;
            digest[..NODE] == *entry(&self.tree, self.plan.levels[level - 1], position)
        };
        if !good {
            return Err(IntegrityError::TreeMismatch { page: index });
        }
        self.tree.extend_from_slice(page);
        self.next += 1;
        Ok(())
    }

    fn finish(self) -> HashTree {
        HashTree { plan: self.plan, tree: self.tree }
    }
}

/// Filled in by a [`VerifyingReader`] once it has read and checked the whole hash tree.
#[derive(Clone, Default, Debug)]
pub struct TreeHandle(Arc<OnceLock<Arc<HashTree>>>);

impl TreeHandle {
    pub fn get(&self) -> Option<Arc<HashTree>> {
        self.0.get().cloned()
    }
}

/// Reads a package from the start, a page at a time, and hands a page on only once it is checked: the header against
/// the trusted copy, the hash tree level by level from the header's top hash, and every hashed page after it against
/// its leaf. The pages between the header and the tree (an embedded XVD, the mutable data page) are outside the tree
/// and passed on as they are; nothing reads them. Any mismatch is final: it and every later read fail with an
/// [`IntegrityError`]. Hashing a page as it arrives costs no second read of anything.
pub struct VerifyingReader<R> {
    upstream: R,
    builder: Option<TreeBuilder>,
    plan: Plan,
    tree: Option<Arc<HashTree>>,
    handle: TreeHandle,
    page: Box<[u8; PAGE_SIZE]>,
    filled: usize,
    verified: bool,
    served: usize,
    index: u64,
    failed: Option<IntegrityError>,
    /// An upstream failure met after some checked bytes were handed on in the same read: reported by the next one.
    deferred: Option<io::Error>,
}

impl<R> VerifyingReader<R> {
    /// `upstream` must yield the package from its first byte; `plan` comes from the trusted header.
    pub fn new(upstream: R, plan: Plan) -> (Self, TreeHandle) {
        let handle = TreeHandle::default();
        let reader = Self {
            upstream,
            builder: Some(TreeBuilder::new(plan.clone())),
            plan,
            tree: None,
            handle: handle.clone(),
            page: Box::new([0u8; PAGE_SIZE]),
            filled: 0,
            verified: false,
            served: 0,
            index: 0,
            failed: None,
            deferred: None,
        };
        (reader, handle)
    }

    fn check(&mut self) -> Result<(), IntegrityError> {
        let offset = self.index * PAGE_SIZE as u64;
        if offset < HEADER_LEN as u64 {
            let at = offset as usize;
            if self.page[..] != self.plan.header[at..at + PAGE_SIZE] {
                return Err(IntegrityError::HeaderMismatch);
            }
        } else if offset < self.plan.tree_start {
            // the embedded XVD and the mutable data are outside the tree, and nothing reads them
        } else if offset < self.plan.tree_end() {
            let builder = self.builder.as_mut().ok_or(IntegrityError::NoTree)?;
            builder.push(&self.page)?;
            if builder.done() {
                let tree = Arc::new(self.builder.take().expect("present").finish());
                let _ = self.handle.0.set(tree.clone());
                self.tree = Some(tree);
            }
        } else {
            self.tree.as_ref().ok_or(IntegrityError::NoTree)?.verify_page(offset, &self.page)?;
        }
        Ok(())
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for VerifyingReader<R> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if let Some(e) = &this.failed {
            return Poll::Ready(Err(e.clone().into()));
        }
        if let Some(e) = this.deferred.take() {
            return Poll::Ready(Err(e));
        }
        // a read that fails must hand on nothing, so a failure after checked bytes waits for the next read
        let mut handed = false;
        loop {
            if buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }
            if this.verified {
                let n = (PAGE_SIZE - this.served).min(buf.remaining());
                buf.put_slice(&this.page[this.served..this.served + n]);
                this.served += n;
                handed = true;
                if this.served == PAGE_SIZE {
                    this.verified = false;
                    this.served = 0;
                    this.filled = 0;
                    this.index += 1;
                }
                continue;
            }
            if this.index * PAGE_SIZE as u64 >= this.plan.len {
                return Poll::Ready(Ok(()));
            }
            while this.filled < PAGE_SIZE {
                let mut read = ReadBuf::new(&mut this.page[this.filled..]);
                match Pin::new(&mut this.upstream).poll_read(cx, &mut read) {
                    // hand on what is checked already rather than wait with it
                    Poll::Pending if handed => return Poll::Ready(Ok(())),
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(e)) if handed => {
                        this.deferred = Some(e);
                        return Poll::Ready(Ok(()));
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Ready(Ok(())) if read.filled().is_empty() => {
                        let e = io::Error::new(io::ErrorKind::UnexpectedEof, "the package ended before its last page");
                        if handed {
                            this.deferred = Some(e);
                            return Poll::Ready(Ok(()));
                        }
                        return Poll::Ready(Err(e));
                    }
                    Poll::Ready(Ok(())) => this.filled += read.filled().len(),
                }
            }
            if let Err(e) = this.check() {
                this.failed = Some(e.clone());
                return Poll::Ready(if handed { Ok(()) } else { Err(e.into()) });
            }
            this.verified = true;
        }
    }
}

/// Builds small, entirely synthetic packages for tests: no game bytes, no real keys.
#[cfg(any(test, feature = "synthetic"))]
#[doc(hidden)]
pub mod synthetic {
    use super::*;
    use crate::crypt::{TweakGenerator, encrypt_page_xts};
    use crate::models::xvd::XvcRegionId;
    use aes::Aes128Enc;
    use aes::cipher::KeyInit;
    use uuid::Uuid;

    pub const KEY: [u8; 32] = [3; 32];
    pub const VDUID: Uuid = Uuid::from_u128(0x7792d9ce_355a_493c_afbd_768f4a77c3b0);
    pub const ENCRYPTED_REGION: u32 = 0x3ea;

    /// A package: header, hash tree, one page of user data, one page of XVC info whose region table lays out the
    /// drive as `plain` plain pages then `encrypted` pages under key 0, and the drive.
    pub struct Package {
        pub bytes: Vec<u8>,
        pub plain_offset: u64,
        pub encrypted_offset: u64,
        /// Byte offset of the region table in the XVC info page.
        pub region_table: u64,
        pub plaintext: Vec<u8>,
    }

    impl Package {
        pub fn header(&self) -> [u8; HEADER_LEN] {
            self.bytes[..HEADER_LEN].try_into().unwrap()
        }
    }

    fn put(buf: &mut [u8], at: usize, value: &[u8]) {
        buf[at..at + value.len()].copy_from_slice(value);
    }

    /// See [`Package`]. `encrypted_plain` = true writes the encrypted region's pages as plaintext and leaves the
    /// region without a key: what an attacker who rewrote the region table would serve.
    pub fn package(plain: u32, encrypted: u32, encrypted_plain: bool) -> Package {
        let drive_pages = plain + encrypted;
        let user_data = PAGE_SIZE as u64;
        let xvc_data = PAGE_SIZE as u64;
        let mut header = [0u8; HEADER_LEN];
        header[..0x200].fill(0x5a); // a stand-in signature
        put(&mut header, 0x200, b"msft-xvd");
        put(&mut header, 0x208, &0x40u32.to_le_bytes()); // REGION_ID_IN_XTS, integrity on
        put(&mut header, 0x20c, &3u32.to_le_bytes());
        put(&mut header, 0x218, &(drive_pages as u64 * PAGE_SIZE as u64).to_le_bytes());
        put(&mut header, 0x220, &VDUID.to_bytes_le());
        put(&mut header, 0x230, &VDUID.to_bytes_le());
        put(&mut header, 0x284, &1u32.to_le_bytes()); // a title
        put(&mut header, 0x28c, &(user_data as u32).to_le_bytes());
        put(&mut header, 0x290, &(xvc_data as u32).to_le_bytes());
        let plan = Plan::new(&header).unwrap();
        let len = plan.len as usize;
        let mut bytes = vec![0u8; len];
        put(&mut bytes, 0, &header);
        let hashed_start = plan.hashed_start as usize;
        let drive_start = plan.drive_start as usize;
        let plain_offset = drive_start as u64;
        let encrypted_offset = plain_offset + plain as u64 * PAGE_SIZE as u64;

        // user data: a header whose type lists no package files
        put(&mut bytes, hashed_start, &16u32.to_le_bytes());
        put(&mut bytes, hashed_start + 8, &1u32.to_le_bytes());
        // XVC info: key 0's id, version 2, two regions
        let info = hashed_start + PAGE_SIZE;
        put(&mut bytes, info, &VDUID.to_bytes_le());
        put(&mut bytes, info + 16, &[0x91; 16]);
        put(&mut bytes, info + 16 + 0xc0 * 16 + 0x100, &2u32.to_le_bytes());
        put(&mut bytes, info + 16 + 0xc0 * 16 + 0x104, &2u32.to_le_bytes());
        let table = info + 3496;
        let region = |bytes: &mut Vec<u8>, at: usize, id: u32, key: u16, offset: u64, pages: u32| {
            put(bytes, at, &id.to_le_bytes());
            put(bytes, at + 4, &key.to_le_bytes());
            put(bytes, at + 80, &offset.to_le_bytes());
            put(bytes, at + 88, &(pages as u64 * PAGE_SIZE as u64).to_le_bytes());
        };
        region(&mut bytes, table, 0x40000003, 0xffff, plain_offset, plain);
        let key = if encrypted_plain { 0xffff } else { 0 };
        region(&mut bytes, table + 128, ENCRYPTED_REGION, key, encrypted_offset, encrypted);

        // the drive: recognisable plaintext
        let mut plaintext = vec![0u8; drive_pages as usize * PAGE_SIZE];
        for (i, b) in plaintext.iter_mut().enumerate() {
            *b = (i / PAGE_SIZE) as u8 ^ (i as u8).wrapping_mul(31);
        }
        put(&mut bytes, drive_start, &plaintext);

        // level 0: every hashed page; the encrypted ones get a data unit and their ciphertext's hash
        let hashed_pages = plan.hashed_pages as usize;
        let mut leaves = vec![[0u8; NODE]; hashed_pages];
        let tweak_key = Aes128Enc::new((&[3u8; 16]).into());
        let data_key = Aes128Enc::new((&[3u8; 16]).into());
        for (i, leaf) in leaves.iter_mut().enumerate() {
            let at = hashed_start + i * PAGE_SIZE;
            let page: &mut [u8; PAGE_SIZE] = (&mut bytes[at..at + PAGE_SIZE]).try_into().unwrap();
            if !encrypted_plain && at as u64 >= encrypted_offset {
                let unit = 1000 + i as u32;
                let tweak = TweakGenerator::new(XvcRegionId::Other(ENCRYPTED_REGION), VDUID).with_data_unit(unit);
                encrypt_page_xts(page, tweak, &tweak_key, &data_key);
                leaf[..LEAF].copy_from_slice(&sha256(page)[..LEAF]);
                leaf[LEAF..].copy_from_slice(&unit.to_le_bytes());
            } else {
                leaf.copy_from_slice(&sha256(page)[..NODE]);
            }
        }
        // the levels, bottom up, each page's hash into the level above
        let tree_at = plan.tree_start as usize;
        let mut below: Vec<[u8; NODE]> = leaves;
        for &(start, pages) in plan.levels.iter().rev() {
            let mut level = vec![0u8; pages as usize * PAGE_SIZE];
            for (i, e) in below.iter().enumerate() {
                let at = (i / PER_PAGE as usize) * PAGE_SIZE + (i % PER_PAGE as usize) * NODE;
                level[at..at + NODE].copy_from_slice(e);
            }
            put(&mut bytes, tree_at + start as usize * PAGE_SIZE, &level);
            below = level.chunks(PAGE_SIZE).map(|p| sha256(p)[..NODE].try_into().unwrap()).collect();
        }
        let top = &bytes[tree_at..tree_at + PAGE_SIZE];
        let top_hash = sha256(top);
        put(&mut bytes, 0x240, &top_hash);
        Package { bytes, plain_offset, encrypted_offset, region_table: table as u64, plaintext }
    }
}

#[cfg(test)]
mod tests {
    use super::synthetic::package;
    use super::*;

    async fn stream(bytes: Vec<u8>, header: [u8; HEADER_LEN]) -> Result<(Vec<u8>, TreeHandle), IntegrityError> {
        let plan = Plan::new(&header)?;
        // in 1000-byte pieces, as a network delivers them
        let reader = tokio_test_chunks(bytes);
        let (mut reader, handle) = VerifyingReader::new(reader, plan);
        let mut out = Vec::new();
        match reader.read_to_end(&mut out).await {
            Ok(_) => Ok((out, handle)),
            Err(e) => Err(find(&e).cloned().unwrap_or_else(|| panic!("not an integrity error: {e}"))),
        }
    }

    fn tokio_test_chunks(bytes: Vec<u8>) -> impl AsyncRead + Unpin {
        let chunks: Vec<io::Result<bytes::Bytes>> =
            bytes.chunks(1000).map(|c| Ok(bytes::Bytes::copy_from_slice(c))).collect();
        tokio_util::io::StreamReader::new(futures_util::stream::iter(chunks))
    }

    #[tokio::test]
    async fn an_intact_package_streams_through_and_yields_its_tree() {
        let p = package(3, 2, false);
        let (out, handle) = stream(p.bytes.clone(), p.header()).await.unwrap();
        assert_eq!(out, p.bytes);
        let tree = handle.get().expect("the tree is complete");
        tree.verify_page(p.plain_offset, p.bytes[p.plain_offset as usize..][..PAGE_SIZE].try_into().unwrap()).unwrap();
        // the local copy loads and checks the same way
        let loaded = HashTree::load(&mut std::io::Cursor::new(p.bytes.clone())).await.unwrap();
        assert_eq!(loaded.header_bytes(), &p.header());
    }

    #[tokio::test]
    async fn a_tree_of_several_levels_is_checked_top_down() {
        // 400 drive pages: 402 hashed pages need three level-0 pages under one level-1 page
        let p = package(300, 100, false);
        let plan = Plan::new(&p.header()).unwrap();
        assert_eq!(plan.levels.len(), 2);
        stream(p.bytes.clone(), p.header()).await.unwrap();
        // a byte of the second level-0 page: its entry in level 1 no longer matches
        let mut bad = p.bytes.clone();
        let at = plan.tree_start as usize + 2 * PAGE_SIZE + 7;
        bad[at] ^= 1;
        assert_eq!(stream(bad, p.header()).await.unwrap_err(), IntegrityError::TreeMismatch { page: 2 });
    }

    #[tokio::test]
    async fn every_kind_of_tampering_is_refused() {
        let p = package(3, 2, false);
        let plan = Plan::new(&p.header()).unwrap();
        let flip = |at: u64| {
            let mut bad = p.bytes.clone();
            bad[at as usize] ^= 0x80;
            bad
        };
        // a plain page (where the game's program lies in a real package)
        let at = p.plain_offset + PAGE_SIZE as u64 + 100;
        assert_eq!(
            stream(flip(at), p.header()).await.unwrap_err(),
            IntegrityError::PageMismatch { offset: p.plain_offset + PAGE_SIZE as u64 }
        );
        // an encrypted page
        assert_eq!(
            stream(flip(p.encrypted_offset + 5), p.header()).await.unwrap_err(),
            IntegrityError::PageMismatch { offset: p.encrypted_offset }
        );
        // the hash tree
        assert_eq!(
            stream(flip(plan.tree_start + 40), p.header()).await.unwrap_err(),
            IntegrityError::TreeMismatch { page: 0 }
        );
        // the header as served, against the trusted copy
        assert_eq!(stream(flip(0x250), p.header()).await.unwrap_err(), IntegrityError::HeaderMismatch);
        // the region table: a key id
        assert_eq!(
            stream(flip(p.region_table + 128 + 4), p.header()).await.unwrap_err(),
            IntegrityError::PageMismatch { offset: plan.hashed_start + PAGE_SIZE as u64 }
        );
        // a package rebuilt whole with its encrypted region declared plain: self-consistent, but its header (with
        // its new top hash) is not the one fetched over HTTPS
        let forged = package(3, 2, true);
        assert_eq!(stream(forged.bytes.clone(), p.header()).await.unwrap_err(), IntegrityError::HeaderMismatch);
        // a cut-off package
        let mut short = p.bytes.clone();
        short.truncate(short.len() - 10);
        let plan = Plan::new(&p.header()).unwrap();
        let (mut reader, _) = VerifyingReader::new(tokio_test_chunks(short), plan);
        let e = reader.read_to_end(&mut Vec::new()).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn a_mismatch_is_final() {
        let p = package(1, 1, false);
        let mut bad = p.bytes.clone();
        bad[p.plain_offset as usize] ^= 1;
        let (mut reader, handle) = VerifyingReader::new(tokio_test_chunks(bad), Plan::new(&p.header()).unwrap());
        let mut out = vec![0; p.bytes.len()];
        assert!(reader.read_exact(&mut out).await.is_err());
        assert!(reader.read(&mut out).await.is_err(), "a refused stream stays refused");
        // the tree itself was intact and came before the bad page
        assert!(handle.get().is_some());
    }

    #[test]
    fn unverifiable_layouts_are_refused() {
        let p = package(1, 1, false);
        let with_flags = |flags: u32| {
            let mut h = p.header();
            h[0x208..0x20c].copy_from_slice(&flags.to_le_bytes());
            Plan::new(&h).map(|_| ())
        };
        assert_eq!(with_flags(0x40), Ok(()));
        assert!(matches!(with_flags(0x44), Err(IntegrityError::Unsupported(_))), "integrity disabled");
        assert!(matches!(with_flags(0x50), Err(IntegrityError::Unsupported(_))), "resilient");
        let mut dynamic = p.header();
        dynamic[0x280] = 1;
        assert!(matches!(Plan::new(&dynamic), Err(IntegrityError::Unsupported(_))));
        let mut magic = p.header();
        magic[0x200] = b'X';
        assert!(matches!(Plan::new(&magic), Err(IntegrityError::Header(_))));
    }
}
