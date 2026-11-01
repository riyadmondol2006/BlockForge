use std::cmp::min;
use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{self, Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::sync::Arc;

use aes::cipher::KeyInit;
use aes::{Aes128Dec, Aes128Enc};
use futures_util::StreamExt;
use msixvc_common::parse::{BinaryParse, BinaryTryParse};
use reqwest::header::RANGE;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::task::block_in_place;
use tokio::time::timeout;
use tokio_util::io::SyncIoBridge;
use uuid::Uuid;
use zerocopy::IntoBytes;

use crate::crypt::{TweakGenerator, decrypt_page_xts};
use crate::integrity::{HashTree, IntegrityError, LEAF};
use crate::layout::{Bytes, PAGE_SIZE, PAGES_PER_BLOCK, Pages};
use crate::math::calculate_hash_block_num_and_run_for_block_num;
use crate::models::xvd::layout::XvdLayout;
use crate::models::xvd::{
    XvcInfo, XvcRegionHeader, XvcRegionId, XvdHashEntry, XvdHeader, XvdSegmentMetadataHeader,
    XvdSegmentMetadataSegment, XvdSegmentMetadataSegmentFlags, XvdUserDataHeader, XvdUserDataPackageFileEntry,
    XvdUserDataPackageFilesHeader,
};
use crate::streaming_ntfs::collect_ntfs_stream_layouts;

pub struct SyncSubstream<R> {
    inner: R,
    start: u64,
    len: u64,
    pos: u64,
}

impl<R> Debug for SyncSubstream<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncSubstream")
            .field("start", &self.start)
            .field("len", &self.len)
            .field("pos", &self.pos)
            .finish_non_exhaustive()
    }
}

impl<R> SyncSubstream<R> {
    pub fn new(inner: R, start: u64, len: u64) -> Self {
        Self { inner, start, len, pos: 0 }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn into_inner(self) -> R {
        self.inner
    }

    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }
}

impl<R: Read + Seek> Read for SyncSubstream<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }

        let remaining = usize::try_from(self.len - self.pos)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "remaining range too large"))?;
        let to_read = remaining.min(buf.len());

        self.inner.seek(SeekFrom::Start(self.start + self.pos))?;
        let read = self.inner.read(&mut buf[..to_read])?;
        self.pos += read as u64;
        Ok(read)
    }
}

impl<R: Seek> Seek for SyncSubstream<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(delta) => {
                if delta >= 0 {
                    self.pos
                        .checked_add(delta as u64)
                        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid relative seek"))?
                } else {
                    self.pos
                        .checked_sub(delta.unsigned_abs())
                        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid relative seek"))?
                }
            }
            SeekFrom::End(delta) => {
                if delta >= 0 {
                    self.len
                        .checked_add(delta as u64)
                        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid end-relative seek"))?
                } else {
                    self.len
                        .checked_sub(delta.unsigned_abs())
                        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid end-relative seek"))?
                }
            }
        };

        if next > self.len {
            return Err(Error::new(ErrorKind::InvalidInput, "seek past substream end"));
        }

        self.pos = next;
        Ok(self.pos)
    }
}

impl<R: Write + Seek> Write for SyncSubstream<R> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }

        let remaining = usize::try_from(self.len - self.pos)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "remaining range too large"))?;
        let to_write = remaining.min(buf.len());

        self.inner.seek(SeekFrom::Start(self.start + self.pos))?;
        let written = self.inner.write(&buf[..to_write])?;
        self.pos += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct XvdEncryptionInfo {
    encrypted_sections: Vec<EncryptedSectionInfo>,
}

// The gpt crate requires the device to implement Debug,
// but the content key must not be debuged
impl Debug for XvdEncryptionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XvdEncryptionInfo")
            .field("encrypted_sections", &self.encrypted_sections)
            .finish_non_exhaustive() // prints ", .." to signal redacted fields
    }
}

struct XvdStream<R> {
    inner: R,
    offset: u64,
    end_offset: u64,

    encryption_info: Option<XvdEncryptionInfo>,
}

impl<R> Debug for XvdStream<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XvdStream")
            .field("offset", &self.offset)
            .field("end_offset", &self.end_offset)
            .field("encryption_info", &self.encryption_info)
            .finish_non_exhaustive()
    }
}

impl<R> XvdStream<R> {
    fn len(&self) -> u64 {
        self.end_offset - self.offset
    }

    fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Seek> XvdStream<R> {
    fn current_relative_pos(&mut self) -> std::io::Result<u64> {
        let absolute = self.inner.stream_position()?;
        absolute
            .checked_sub(self.offset)
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "stream before virtual start"))
    }
}

impl<R: Read + Seek> Read for XvdStream<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let current = self.current_relative_pos()?;
        if current >= self.len() {
            return Ok(0);
        }

        let remaining = usize::try_from(self.len() - current)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "remaining range too large"))?;
        let to_read = remaining.min(buf.len());

        self.inner.read(&mut buf[..to_read])
    }
}

impl<R: Seek> Seek for XvdStream<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_relative = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(delta) => {
                let current = self.current_relative_pos()?;
                if delta >= 0 { current.checked_add(delta as u64) } else { current.checked_sub(delta.unsigned_abs()) }
                    .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid relative seek"))?
            }
            SeekFrom::End(delta) => {
                let len = self.len();
                if delta >= 0 { len.checked_add(delta as u64) } else { len.checked_sub(delta.unsigned_abs()) }
                    .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "invalid end-relative seek"))?
            }
        };

        if new_relative > self.len() {
            return Err(Error::new(ErrorKind::InvalidInput, "seek past virtual device end"));
        }

        self.inner.seek(SeekFrom::Start(self.offset + new_relative))?;
        Ok(new_relative)
    }
}

impl<R> Write for XvdStream<R> {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(Error::new(ErrorKind::PermissionDenied, "XvdStream is read-only"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub struct XvdFile {
    header: XvdHeader,
    layout: XvdLayout,
    encrypted_section_infos: Vec<EncryptedSectionInfo>,
    /// BlockForge: the verified hash tree every extracted page is checked against; extraction refuses without it.
    integrity: Option<Arc<HashTree>>,
}

#[derive(Debug)]
pub struct EncryptedSectionInfo {
    section_offset: u64,
    section_length: u64,

    header_id: XvcRegionId,
    vduid: Uuid,

    // If integrity is enabled, this must contain one entry per page in the section.
    // If integrity is disabled, use page_in_section as the data unit instead.
    data_units: Option<Vec<u32>>,
    first_segment_index: u32,
    data_hashs: Vec<[u8; 20]>,
}

pub struct UserPackageFile {
    pub offset: u64,
    pub length: u64,
}

pub struct SegmentFile {
    pub offset: u64,
    pub length: u64,
    pub data_hashs: Vec<[u8; 20]>,
    pub keep_encrypted: bool,
}

/// See [`XvdFile::segment_encryption`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentEncryption {
    Encrypted,
    Plain,
    Straddles,
}

impl XvdFile {
    /// Where `file` lies against the package's encrypted sections, as decrypting reads it (`extract_file_ex` picks
    /// the section its first byte is in, else copies the pages as they are): the adapter decrypts the first, copies
    /// the second, and refuses the third, which would copy encrypted pages as if they were plain.
    pub fn segment_encryption(&self, file: &SegmentFile) -> SegmentEncryption {
        let Some(end) = file.offset.checked_add(file.length) else { return SegmentEncryption::Straddles };
        let within = |s: &EncryptedSectionInfo| {
            file.offset >= s.section_offset && file.offset < s.section_offset.saturating_add(s.section_length)
        };
        if self.encrypted_section_infos.iter().any(within) {
            return SegmentEncryption::Encrypted;
        }
        let touches = self
            .encrypted_section_infos
            .iter()
            .any(|s| file.offset < s.section_offset.saturating_add(s.section_length) && s.section_offset < end);
        if touches { SegmentEncryption::Straddles } else { SegmentEncryption::Plain }
    }
    pub fn content_id(&self) -> uuid::Uuid {
        self.header.vduid
    }

    pub fn header(&self) -> &XvdHeader {
        &self.header
    }

    /// BlockForge: check every page extracted from now on against `tree`, which must have been verified from this
    /// package's header (see [`crate::integrity`]).
    pub fn require_integrity(&mut self, tree: Arc<HashTree>) -> Result<(), IntegrityError> {
        if tree.header() != &self.header {
            return Err(IntegrityError::HeaderMismatch);
        }
        self.integrity = Some(tree);
        Ok(())
    }

    /// BlockForge: the tree's 20-byte hashes of every page of `file`, as stored in the package.
    pub fn page_hashes(&self, file: &SegmentFile) -> Result<Vec<[u8; LEAF]>, IntegrityError> {
        let tree = self.integrity.as_ref().ok_or(IntegrityError::NoTree)?;
        let pages = file.length.div_ceil(PAGE_SIZE as u64);
        (0..pages)
            .map(|i| {
                let offset = i
                    .checked_mul(PAGE_SIZE as u64)
                    .and_then(|o| o.checked_add(file.offset))
                    .ok_or(IntegrityError::Unhashed { offset: file.offset })?;
                tree.leaf(offset)
            })
            .collect()
    }

    /// BlockForge: the page of the package at byte `offset`, as stored, against the verified tree.
    fn verify_page(&self, offset: u64, page: &[u8; PAGE_SIZE]) -> Result<(), IntegrityError> {
        self.integrity.as_ref().ok_or(IntegrityError::NoTree)?.verify_page(offset, page)
    }

    /// BlockForge: checks a page of the package as it was written out, decrypted where it lies in an encrypted
    /// section (all but the files kept encrypted): it is encrypted again with `full_key` and the data unit the
    /// verified tree gives it, and that ciphertext is checked. Lets a build on disk be checked offline with its key.
    pub fn verify_plaintext_page(
        &self,
        offset: u64,
        page: &[u8; PAGE_SIZE],
        full_key: [u8; 32],
    ) -> Result<(), IntegrityError> {
        let Some(s) = self
            .encrypted_section_infos
            .iter()
            .find(|s| offset >= s.section_offset && offset < s.section_offset.saturating_add(s.section_length))
        else {
            return self.verify_page(offset, page);
        };
        let index = ((offset - s.section_offset) / PAGE_SIZE as u64) as usize;
        let unit = match &s.data_units {
            Some(units) => *units.get(index).ok_or(IntegrityError::Unhashed { offset })?,
            None => index as u32,
        };
        let mut tweak_key = [0u8; 16];
        let mut data_key = [0u8; 16];
        tweak_key.copy_from_slice(&full_key[..16]);
        data_key.copy_from_slice(&full_key[16..]);
        let mut ciphertext = *page;
        crate::crypt::encrypt_page_xts(
            &mut ciphertext,
            TweakGenerator::new(s.header_id, s.vduid).with_data_unit(unit),
            &Aes128Enc::new((&tweak_key).into()),
            &Aes128Enc::new((&data_key).into()),
        );
        self.verify_page(offset, &ciphertext)
    }

    /// BlockForge: a file is read in whole pages from its first byte, so it must start on a page.
    fn check_aligned(sfile: &SegmentFile) -> Result<(), IntegrityError> {
        if !sfile.offset.is_multiple_of(PAGE_SIZE as u64) {
            return Err(IntegrityError::Unhashed { offset: sfile.offset });
        }
        Ok(())
    }

    fn non_encrypted_prefix_len(&self, start: u64, len: u64) -> u64 {
        let end = start.saturating_add(len);
        let mut prefix_len = len;

        for section in &self.encrypted_section_infos {
            let section_start = section.section_offset;
            let section_end = section.section_offset.saturating_add(section.section_length);

            if section_end <= start || section_start >= end {
                continue;
            }

            if start >= section_start {
                return 0;
            }

            prefix_len = section_start.saturating_sub(start);
            break;
        }

        prefix_len
    }

    pub async fn parse_file(path: String) -> Result<Self, Box<dyn std::error::Error>> {
        let mut file = OpenOptions::new().read(true).open(path.clone()).await?;
        Self::parse(&mut file).await
    }

    pub async fn parse<Reader>(mut file: Reader) -> Result<Self, Box<dyn std::error::Error>>
    where
        Reader: AsyncRead + AsyncSeek + Unpin,
    {
        let xvd_header = {
            let mut buf = XvdHeader::buffer();
            file.read_exact(&mut buf).await?;
            XvdHeader::try_from_array(&buf)?
        };

        if xvd_header.number_of_hashed_pages().0 == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Empty XVD hash layout").into());
        }
        let layout = xvd_header.layout();

        let mut region_headers: Vec<XvcRegionHeader> = Vec::new();

        // TODO: Check if we have proper content type
        if layout.xvc_info.len > Bytes(0) {
            file.seek(std::io::SeekFrom::Start(layout.xvc_info.start.to_bytes().0)).await?;

            let xvc_info = {
                let mut buf = XvcInfo::buffer();
                file.read_exact(&mut buf).await?;
                XvcInfo::from_array(&buf)
            };

            let region_count = xvc_info.region_count;

            if xvc_info.version >= 1 {
                // a region header is 128 bytes, and they all lie in the XVC info
                if region_count as u64 * 128 > layout.xvc_info.len.0 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Region table larger than XVC info").into());
                }
                let mut buf = XvcRegionHeader::buffer();
                for _ in 0..region_count {
                    file.read_exact(&mut buf).await?;
                    let region_header = XvcRegionHeader::try_from_array(&buf)?;
                    region_headers.push(region_header);
                }
            }
        }

        let mut enc_sections: Vec<EncryptedSectionInfo> = vec![];
        let mut reader = BufReader::with_capacity(PAGES_PER_BLOCK * XvdHashEntry::SIZE, file);
        for h in region_headers {
            let key_id = h.key_id;
            let num_pages = h.length.0;
            match key_id.get() {
                None => continue,
                Some(0) => (),
                // BlockForge: an error, not upstream's todo!() panic; the licence carries one content key
                Some(n) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Encryption key {n} is not supported (only key 0)"),
                    )
                    .into());
                }
            }

            let hashed = xvd_header.number_of_hashed_pages().0;
            let start_page = h
                .offset
                .0
                .checked_sub(layout.user_data.start.0)
                .filter(|start| start.checked_add(num_pages).is_some_and(|end| end <= hashed))
                .map(Pages)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Encrypted region outside the hashed pages")
                })?;
            let mut data_units: Vec<u32> = Vec::with_capacity(num_pages as usize);
            let mut data_hashs: Vec<[u8; 20]> = Vec::with_capacity(num_pages as usize);

            let mut page = 0;
            loop {
                if page >= num_pages {
                    break;
                }
                let (hash_block, entry_start, run_length) = calculate_hash_block_num_and_run_for_block_num(
                    xvd_header.xvd_type as u32,
                    layout.hash_tree_layout.top_level() as u64 + 1,
                    xvd_header.number_of_hashed_pages(),
                    start_page.0 + page,
                    0,
                    false,
                    false,
                );
                let run_length = min(run_length, num_pages - page);
                page += run_length;
                let read_offset = layout.hash_tree.start.to_bytes()
                    + Pages(hash_block).to_bytes()
                    + Bytes(entry_start as u64 * XvdHashEntry::SIZE as u64);
                reader.seek(SeekFrom::Start(read_offset.0)).await?;

                let mut buf = XvdHashEntry::buffer();
                for _ in 0..run_length {
                    reader.read_exact(&mut buf).await?;
                    let hash = XvdHashEntry::from_array(&buf);
                    data_units.push(hash.unit);
                    data_hashs.push(hash.block_hash);
                }
            }

            enc_sections.push(EncryptedSectionInfo {
                section_offset: h.offset.to_bytes().0,
                section_length: h.length.to_bytes().0,
                header_id: h.region_id,
                vduid: xvd_header.vduid,
                data_units: Some(data_units),
                first_segment_index: h.first_segment_index,
                data_hashs,
            });
        }
        Ok(XvdFile { header: xvd_header, layout, encrypted_section_infos: enc_sections, integrity: None })
    }

    pub async fn parse_user_package_files<Reader>(
        &self,
        mut file: Reader,
    ) -> Result<HashMap<String, UserPackageFile>, Box<dyn std::error::Error>>
    where
        Reader: AsyncRead + AsyncSeek + Unpin,
    {
        let mut files = HashMap::new();

        let user_data_offset = self.layout.user_data.start.0 as u64;
        file.seek(SeekFrom::Start(user_data_offset)).await?;
        let user_data_header = {
            let mut buf = XvdUserDataHeader::buffer();
            file.read_exact(&mut buf).await?;
            XvdUserDataHeader::from_array(&buf)
        };
        if user_data_header.t == 0 {
            let mut off = user_data_offset + user_data_header.length as u64;
            file.seek(SeekFrom::Start(off)).await?;
            let user_data_package_files_header = {
                let mut buf = XvdUserDataPackageFilesHeader::buffer();
                file.read_exact(&mut buf).await?;
                XvdUserDataPackageFilesHeader::from_array(&buf)
            };
            off += XvdUserDataPackageFilesHeader::SIZE as u64;
            let mut buf = XvdUserDataPackageFileEntry::buffer();
            for _ in 0..user_data_package_files_header.file_count {
                file.seek(SeekFrom::Start(off)).await?;
                file.read_exact(&mut buf).await?;
                let user_data_package_file_entry = XvdUserDataPackageFileEntry::from_array(&buf);
                off += XvdUserDataPackageFileEntry::SIZE as u64;
                let o = user_data_package_file_entry.offset;
                let s: u32 = user_data_package_file_entry.size;
                let fullname = user_data_package_file_entry.file_path;
                let end = fullname.iter().position(|&c| c == 0).unwrap_or(fullname.len());
                let pfull_name: String = String::from_utf16(&fullname[..end])
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid package file name"))?;

                files.insert(
                    pfull_name,
                    UserPackageFile {
                        offset: user_data_offset + XvdUserDataHeader::SIZE as u64 + o as u64,
                        length: s as u64,
                    },
                );
            }
        }
        Ok(files)
    }

    pub async fn parse_segment_metadata<Reader>(
        &self,
        file: Reader,
        segment_metadata: &UserPackageFile,
    ) -> Result<HashMap<String, SegmentFile>, Box<dyn std::error::Error>>
    where
        Reader: AsyncRead + AsyncSeek + Unpin,
    {
        // BlockForge: the length comes from the package; a capped buffer, not one of up to 4 GiB
        let mut file = BufReader::with_capacity((segment_metadata.length as usize).clamp(4096, 1 << 20), file);
        file.seek(SeekFrom::Start(segment_metadata.offset)).await?;
        let segment_header = {
            let mut buf = XvdSegmentMetadataHeader::buffer();
            file.read_exact(&mut buf).await?;
            XvdSegmentMetadataHeader::try_from_array(&buf)?
        };
        let paths_offset = segment_header.header_length as u64 + segment_header.segment_count as u64 * 0x10;

        let mut segments = Vec::with_capacity(segment_header.segment_count as usize);
        let mut buf = XvdSegmentMetadataSegment::buffer();
        for _ in 0..segment_header.segment_count {
            file.read_exact(&mut buf).await?;
            let segment = XvdSegmentMetadataSegment::from_array(&buf);
            segments.push(segment);
        }

        let mut files = HashMap::new();

        for section in &self.encrypted_section_infos {
            let segment_page_start = section.section_offset.div_ceil(PAGE_SIZE as u64);
            let mut page_offset = segment_page_start;
            for segment_no in section.first_segment_index..segment_header.segment_count {
                let segment = segments
                    .get(segment_no as usize)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Segment index out of range"))?;
                let s = segment.path_length;
                let mut buf = vec![0u16, 0];
                buf.resize(s as usize, 0);
                file.seek(SeekFrom::Start(segment_metadata.offset + paths_offset + segment.path_offset as u64)).await?;
                file.read_exact(buf.as_mut_bytes()).await?;
                let file_name: String = String::from_utf16(buf.as_slice())
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid segment file name"))?;
                let page_length = if segment.filesize == 0 { 1 } else { segment.filesize.div_ceil(PAGE_SIZE as u64) };
                if page_offset * (PAGE_SIZE as u64) >= section.section_offset + section.section_length {
                    break;
                }
                let end = page_offset as usize - segment_page_start as usize
                    + segment.filesize.div_ceil(PAGE_SIZE as u64) as usize;
                let data_hashs: Vec<[u8; 20]> = section
                    .data_hashs
                    .get(page_offset as usize - segment_page_start as usize..end)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Segment beyond its section's hashes"))?
                    .into();
                files.insert(
                    file_name,
                    SegmentFile {
                        offset: page_offset * PAGE_SIZE as u64,
                        length: segment.filesize,
                        data_hashs,
                        keep_encrypted: segment.flags.contains(XvdSegmentMetadataSegmentFlags::KEEP_ENCRYPTED_ON_DISK),
                    },
                );
                page_offset += page_length;
            }
        }
        Ok(files)
    }

    pub fn populate_segment_hashes(
        &self,
        files: &mut HashMap<String, SegmentFile>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for (name, file) in files.iter_mut() {
            if !file.data_hashs.is_empty() {
                continue;
            }

            let Some(section) = self.encrypted_section_infos.iter().find(|section| {
                file.offset >= section.section_offset && file.offset < section.section_offset + section.section_length
            }) else {
                continue;
            };

            let file_end = file.offset.saturating_add(file.length);
            let section_end = section.section_offset.saturating_add(section.section_length);
            if file_end > section_end {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "segment file spans beyond encrypted section: {} ({}..{} > {}..{})",
                        name, file.offset, file_end, section.section_offset, section_end
                    ),
                )
                .into());
            }

            let segment_page_start = section.section_offset.div_ceil(PAGE_SIZE as u64);
            let page_offset = file.offset.div_ceil(PAGE_SIZE as u64);
            let page_count = file.length.div_ceil(PAGE_SIZE as u64) as usize;
            let start = page_offset.checked_sub(segment_page_start).ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("segment page offset before section start: {} ({})", name, file.offset),
                )
            })? as usize;
            let end = start + page_count;

            if end > section.data_hashs.len() {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "missing data hashes for {}: need [{}..{}], have {}",
                        name,
                        start,
                        end,
                        section.data_hashs.len()
                    ),
                )
                .into());
            }

            file.data_hashs = section.data_hashs[start..end].into();
        }

        Ok(())
    }

    pub async fn parse_ntfs_segment_metadata<Reader>(
        &self,
        file: Reader,
        only_plain: bool,
    ) -> Result<HashMap<String, SegmentFile>, Box<dyn std::error::Error>>
    where
        Reader: AsyncRead + AsyncSeek + Unpin,
    {
        let drive_data_offset = self.layout.drive_data.start.to_bytes().0;
        let drive_size = self.header.drive_size;
        let drive_plain_len = self.non_encrypted_prefix_len(drive_data_offset, drive_size.0);

        block_in_place(|| {
            let block_size = 4096;
            let drive = SyncSubstream::new(
                XvdStream {
                    inner: SyncIoBridge::new(file),
                    offset: drive_data_offset,
                    end_offset: drive_data_offset + drive_plain_len,
                    encryption_info: None,
                },
                0,
                drive_plain_len,
            );

            let gp = gpt::GptConfig::new()
                .writable(false)
                .logical_block_size(if block_size == 512 {
                    gpt::disk::LogicalBlockSize::Lb512
                } else {
                    gpt::disk::LogicalBlockSize::Lb4096
                })
                .open_from_device(drive)?;

            let (_, part) = gp
                .partitions()
                .iter()
                .find(|(_, part)| part.is_used())
                .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "no used GPT partition found"))?;

            let part_start = part
                .bytes_start(*gp.logical_block_size())
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "Invalid GPT partition start"))?;
            let part_len = part
                .bytes_len(*gp.logical_block_size())
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "Invalid GPT partition length"))?;

            let bridge = gp.take_device().into_inner().into_inner();
            let partition_offset = drive_data_offset + part_start;
            let partition_plain_len = self.non_encrypted_prefix_len(partition_offset, part_len);
            let mut fs = SyncSubstream::new(
                XvdStream {
                    inner: bridge,
                    offset: partition_offset,
                    end_offset: partition_offset + partition_plain_len,
                    encryption_info: None,
                },
                0,
                partition_plain_len,
            );

            let reports = collect_ntfs_stream_layouts(&mut fs)?;
            let mut files = HashMap::new();

            for report in reports {
                if report.path.starts_with('$') || report.path.contains(':') {
                    continue;
                }
                if report.resident_data || report.data_runs.len() != 1 {
                    continue;
                }

                let Some(data_run) = report.data_runs.first() else {
                    continue;
                };
                let Some(start) = data_run.start else {
                    continue;
                };

                if only_plain && partition_offset + start >= drive_data_offset + drive_plain_len {
                    continue;
                }

                files.insert(
                    report.path.replace("/", "\\"),
                    SegmentFile {
                        offset: partition_offset + start,
                        length: report.value_length,
                        data_hashs: vec![],
                        keep_encrypted: !only_plain && report.path.to_ascii_lowercase().ends_with(".exe"),
                    },
                );
            }

            self.populate_segment_hashes(&mut files)?;

            Ok(files)
        })
    }

    pub async fn download_file_http<Writer, Progress>(
        &self,
        client: &reqwest::Client,
        url: &str,
        out: &mut Writer,
        sfile: &SegmentFile,
        full_key: [u8; 32],
        mut progress: Progress,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        Writer: AsyncWrite + Unpin,
        Progress: FnMut(u64, u64),
    {
        if sfile.length == 0 {
            return Ok(());
        }
        Self::check_aligned(sfile)?;

        let s = &self
            .encrypted_section_infos
            .iter()
            .find(|s| sfile.offset >= s.section_offset && sfile.offset < s.section_offset + s.section_length);

        let mut tweak = None;
        let mut tweak_cipher = None;
        let mut data_cipher = None;

        let file_offset_in_section;

        if let Some(s) = s
            && !sfile.keep_encrypted
        {
            let mut tweak_key = [0u8; 16];
            let mut data_key = [0u8; 16];
            tweak_key.copy_from_slice(&full_key[..16]);
            data_key.copy_from_slice(&full_key[16..]);

            tweak = Some(TweakGenerator::new(s.header_id, s.vduid));
            tweak_cipher = Some(Aes128Enc::new((&tweak_key).into()));
            data_cipher = Some(Aes128Dec::new((&data_key).into()));
            file_offset_in_section = sfile.offset - s.section_offset;
        } else {
            // TODO for data integrity we need a section for unencrypted sections...
            file_offset_in_section = sfile.offset;
        }
        let page_start = file_offset_in_section / PAGE_SIZE as u64;
        let page_count = sfile.length.div_ceil(PAGE_SIZE as u64);

        let mut page = [0u8; PAGE_SIZE];
        let mut remaining = sfile.length;
        let mut page_in_section = page_start;
        let page_length = sfile.length.div_ceil(PAGE_SIZE as u64) * PAGE_SIZE as u64;
        let mut stream = None;
        let mut pending = bytes::BytesMut::new();
        let mut v: u64 = 0;

        let stall_timeout = tokio::time::Duration::from_secs(5);
        if let Ok(Ok(Ok(response))) = timeout(
            stall_timeout,
            client
                .get(url)
                .header(RANGE, format!("bytes={}-{}", sfile.offset + v, sfile.offset + page_length - 1))
                .send(),
        )
        .await
        .map(|o| o.map(|o| o.error_for_status()))
            && response.status() == 206
        {
            stream = Some(response.bytes_stream());
        }
        loop {
            if page_in_section >= page_start + page_count || remaining == 0 {
                break;
            }
            let next = if let Some(s) = stream.as_mut() { timeout(stall_timeout, s.next()).await } else { Ok(None) };
            let data;
            if let Ok(Some(Ok(b))) = next {
                data = b;
            } else {
                // error
                if let Ok(Ok(Ok(response))) = timeout(
                    stall_timeout,
                    client
                        .get(url)
                        .header(RANGE, format!("bytes={}-{}", sfile.offset + v, sfile.offset + page_length - 1))
                        .send(),
                )
                .await
                .map(|o| o.map(|o| o.error_for_status()))
                    && response.status() == 206
                {
                    stream = Some(response.bytes_stream());
                    continue;
                }
                continue;
            }

            v += data.len() as u64;
            progress(min(v, sfile.length), sfile.length);

            pending.extend_from_slice(&data);

            while pending.len() >= 4096 {
                if page_in_section >= page_start + page_count || remaining == 0 {
                    break;
                }
                let chunk = pending.split_to(4096);
                page.copy_from_slice(&chunk);
                // BlockForge: as stored, before any decryption
                self.verify_page(sfile.offset + (page_in_section - page_start) * PAGE_SIZE as u64, &page)?;
                let to_write_remaining = remaining.min(PAGE_SIZE as u64) as usize;
                let to_write = if let Some(tweak) = &tweak {
                    let tweak = tweak.with_data_unit(match &s.unwrap().data_units {
                        Some(units) => *units.get(page_in_section as usize).ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                format!(
                                    "{} units {} page_in_section {} ({}+{})",
                                    "missing data unit",
                                    (*units).len(),
                                    page_in_section,
                                    page_start,
                                    page_count
                                ),
                            )
                        })?,
                        None => page_in_section as u32,
                    });
                    decrypt_page_xts(&mut page, tweak, tweak_cipher.as_ref().unwrap(), data_cipher.as_ref().unwrap());
                    to_write_remaining
                } else if sfile.keep_encrypted {
                    // Decryption needs full 4k blocks
                    PAGE_SIZE
                } else {
                    to_write_remaining
                };
                out.write_all(&page[..to_write]).await?;
                remaining -= to_write_remaining as u64;

                page_in_section += 1;
            }
        }
        if remaining > 0 {
            return Err(Box::new(std::io::Error::other(format!(
                "{} of {} missing have {}",
                remaining, sfile.length, v
            ))));
        }
        Ok(())
    }

    async fn extract_file_ex<Writer, Reader, Progress>(
        &self,
        i: &mut Reader,
        out: &mut Writer,
        sfile: &SegmentFile,
        full_key: [u8; 32],
        mut progress: Progress,
        decrypt_all: bool,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        Reader: AsyncRead + Unpin,
        Writer: AsyncWrite + Unpin,
        Progress: FnMut(u64, u64),
    {
        if sfile.length == 0 {
            return Ok(());
        }
        Self::check_aligned(sfile)?;

        let s = &self
            .encrypted_section_infos
            .iter()
            .find(|s| sfile.offset >= s.section_offset && sfile.offset < s.section_offset + s.section_length);

        let mut tweak = None;
        let mut tweak_cipher = None;
        let mut data_cipher = None;

        let file_offset_in_section;

        if let Some(s) = s
            && (!sfile.keep_encrypted || decrypt_all)
        {
            let mut tweak_key = [0u8; 16];
            let mut data_key = [0u8; 16];
            tweak_key.copy_from_slice(&full_key[..16]);
            data_key.copy_from_slice(&full_key[16..]);

            tweak = Some(TweakGenerator::new(s.header_id, s.vduid));
            tweak_cipher = Some(Aes128Enc::new((&tweak_key).into()));
            data_cipher = Some(Aes128Dec::new((&data_key).into()));
            file_offset_in_section = sfile.offset - s.section_offset;
        } else {
            // TODO for data integrity we need a section for unencrypted sections...
            file_offset_in_section = sfile.offset;
        }
        let page_start = file_offset_in_section / PAGE_SIZE as u64;
        let page_count = sfile.length.div_ceil(PAGE_SIZE as u64);

        let mut page = [0u8; PAGE_SIZE];

        for page_in_section in page_start..page_start + page_count {
            progress(min((page_in_section - page_start) * 4096, sfile.length), sfile.length);
            i.read_exact(&mut page).await?;
            // BlockForge: every page, as stored (ciphertext where encrypted), against the verified tree before it is
            // decrypted or written: a download and a staging from the copy on disk alike
            self.verify_page(sfile.offset + (page_in_section - page_start) * PAGE_SIZE as u64, &page)?;
            let to_write = min(
                PAGE_SIZE,
                sfile.length as usize
                    - min((page_in_section - page_start) as usize * 4096_usize, sfile.length as usize),
            ) as usize;
            let to_write = if let Some(tweak) = &tweak {
                let tweak = tweak.with_data_unit(match &s.unwrap().data_units {
                    Some(units) => *units.get(page_in_section as usize).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!(
                                "{} units {} page_in_section {} ({}+{})",
                                "missing data unit",
                                (*units).len(),
                                page_in_section,
                                page_start,
                                page_count
                            ),
                        )
                    })?,
                    None => page_in_section as u32,
                });
                decrypt_page_xts(&mut page, tweak, tweak_cipher.as_ref().unwrap(), data_cipher.as_ref().unwrap());
                to_write
            } else if sfile.keep_encrypted {
                // Decryption needs full 4k blocks
                PAGE_SIZE
            } else {
                to_write
            };
            out.write_all(&page[..to_write]).await?;
        }
        Ok(())
    }

    // Reader is an full xvd file
    pub async fn extract_file<Writer, Reader, Progress>(
        &self,
        i: &mut Reader,
        out: &mut Writer,
        sfile: &SegmentFile,
        full_key: [u8; 32],
        progress: Progress,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        Reader: AsyncRead + AsyncSeek + Unpin,
        Writer: AsyncWrite + Unpin,
        Progress: FnMut(u64, u64),
    {
        i.seek(std::io::SeekFrom::Start(sfile.offset)).await?;
        self.extract_file_ex(i, out, sfile, full_key, progress, false).await
    }

    // Reader points to file content
    pub async fn mount_mem_fd<Writer, Reader, Progress>(
        &self,
        i: &mut Reader,
        out: &mut Writer,
        sfile: &SegmentFile,
        full_key: [u8; 32],
        progress: Progress,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        Reader: AsyncRead + Unpin,
        Writer: AsyncWrite + Unpin,
        Progress: FnMut(u64, u64),
    {
        self.extract_file_ex(i, out, sfile, full_key, progress, true).await
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use crate::integrity::synthetic::{KEY, package};

    async fn verified(bytes: &[u8]) -> XvdFile {
        let mut xvd = XvdFile::parse(std::io::Cursor::new(bytes.to_vec())).await.unwrap();
        let tree = HashTree::load(&mut std::io::Cursor::new(bytes.to_vec())).await.unwrap();
        xvd.require_integrity(tree).unwrap();
        xvd
    }

    #[tokio::test]
    async fn synthetic_xvd_encrypted_segment_retains_pages_and_stages_logical_length() {
        // Entirely synthetic header/data. No game bytes or production keys.
        let p = package(1, 2, false);
        let xvd = verified(&p.bytes).await;
        let plaintext = &p.plaintext[PAGE_SIZE..];
        let segment = SegmentFile { offset: p.encrypted_offset, length: 123, data_hashs: vec![], keep_encrypted: true };
        assert_eq!(xvd.segment_encryption(&segment), SegmentEncryption::Encrypted);
        assert_eq!(xvd.page_hashes(&segment).unwrap().len(), 1);
        let mut ciphertext = Vec::new();
        xvd.extract_file(&mut std::io::Cursor::new(p.bytes.clone()), &mut ciphertext, &segment, KEY, |_, _| {})
            .await
            .unwrap();
        assert_eq!(ciphertext.len(), 4096);
        assert_ne!(&ciphertext[..123], &plaintext[..123]);
        let mut output = Vec::new();
        xvd.mount_mem_fd(&mut std::io::Cursor::new(ciphertext.clone()), &mut output, &segment, KEY, |_, _| {})
            .await
            .unwrap();
        assert_eq!(output, plaintext[..123]);
        // a staged copy whose ciphertext changed on disk is refused, not decrypted into garbage
        ciphertext[9] ^= 1;
        let error = xvd
            .mount_mem_fd(&mut std::io::Cursor::new(ciphertext), &mut Vec::new(), &segment, KEY, |_, _| {})
            .await
            .unwrap_err();
        assert_eq!(crate::integrity::find(&*error), Some(&IntegrityError::PageMismatch { offset: p.encrypted_offset }));
    }

    async fn extract(xvd: &XvdFile, bytes: Vec<u8>, file: &SegmentFile) -> Result<Vec<u8>, Option<IntegrityError>> {
        let mut out = Vec::new();
        xvd.extract_file(&mut std::io::Cursor::new(bytes), &mut out, file, KEY, |_, _| {})
            .await
            .map(|_| out)
            .map_err(|e| crate::integrity::find(&*e).cloned())
    }

    #[tokio::test]
    async fn extraction_checks_every_page_and_needs_a_verified_tree() {
        let p = package(2, 2, false);
        let xvd = verified(&p.bytes).await;
        // a plain file across two pages, decrypted file across two encrypted pages
        let plain = SegmentFile { offset: p.plain_offset, length: 5000, data_hashs: vec![], keep_encrypted: false };
        let encrypted =
            SegmentFile { offset: p.encrypted_offset, length: 6000, data_hashs: vec![], keep_encrypted: false };
        assert_eq!(extract(&xvd, p.bytes.clone(), &plain).await.unwrap(), p.plaintext[..5000]);
        assert_eq!(extract(&xvd, p.bytes.clone(), &encrypted).await.unwrap(), p.plaintext[2 * PAGE_SIZE..][..6000]);
        // a byte changed in the second page of each (past the file's end in the plain one: the page is hashed whole)
        for (file, at) in [(&plain, p.plain_offset + PAGE_SIZE as u64 + 4095), (&encrypted, p.encrypted_offset + 4096)]
        {
            let mut bad = p.bytes.clone();
            bad[at as usize] ^= 4;
            let page = at / PAGE_SIZE as u64 * PAGE_SIZE as u64;
            assert_eq!(
                extract(&xvd, bad, file).await.unwrap_err(),
                Some(IntegrityError::PageMismatch { offset: page })
            );
        }
        // without a tree nothing is extracted; nor from a tree of another package
        let bare = XvdFile::parse(std::io::Cursor::new(p.bytes.clone())).await.unwrap();
        assert_eq!(extract(&bare, p.bytes.clone(), &plain).await.unwrap_err(), Some(IntegrityError::NoTree));
        let other = package(3, 1, false);
        let mut bare = XvdFile::parse(std::io::Cursor::new(p.bytes.clone())).await.unwrap();
        let tree = HashTree::load(&mut std::io::Cursor::new(other.bytes)).await.unwrap();
        assert_eq!(bare.require_integrity(tree).unwrap_err(), IntegrityError::HeaderMismatch);
        // nor a file that doesn't start on a page
        let odd = SegmentFile { offset: p.plain_offset + 1, length: 10, data_hashs: vec![], keep_encrypted: false };
        assert!(extract(&xvd, p.bytes.clone(), &odd).await.is_err());
    }

    #[tokio::test]
    async fn a_decrypted_page_is_checked_by_encrypting_it_again() {
        let p = package(1, 2, false);
        let xvd = verified(&p.bytes).await;
        let page = |at: usize| -> [u8; PAGE_SIZE] { p.plaintext[at..at + PAGE_SIZE].try_into().unwrap() };
        xvd.verify_plaintext_page(p.plain_offset, &page(0), KEY).unwrap();
        xvd.verify_plaintext_page(p.encrypted_offset + PAGE_SIZE as u64, &page(2 * PAGE_SIZE), KEY).unwrap();
        let mut changed = page(PAGE_SIZE);
        changed[0] ^= 1;
        assert!(xvd.verify_plaintext_page(p.encrypted_offset, &changed, KEY).is_err());
        assert!(xvd.verify_plaintext_page(p.encrypted_offset, &page(PAGE_SIZE), [4; 32]).is_err(), "another key");
    }

    #[tokio::test]
    async fn crafted_region_tables_are_errors_not_panics() {
        let p = package(1, 1, false);
        let key_at = p.region_table as usize + 128 + 4;
        let mut other_key = p.bytes.clone();
        other_key[key_at..key_at + 2].copy_from_slice(&5u16.to_le_bytes());
        assert!(XvdFile::parse(std::io::Cursor::new(other_key)).await.is_err());
        let offset_at = p.region_table as usize + 128 + 80;
        let mut before = p.bytes.clone();
        before[offset_at..offset_at + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(XvdFile::parse(std::io::Cursor::new(before)).await.is_err());
        let mut past = p.bytes.clone();
        past[offset_at + 8..offset_at + 16].copy_from_slice(&(1u64 << 40).to_le_bytes());
        assert!(XvdFile::parse(std::io::Cursor::new(past)).await.is_err());
        let count_at = p.region_table as usize - 3496 + 16 + 0xc0 * 16 + 0x104;
        let mut many = p.bytes.clone();
        many[count_at..count_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(XvdFile::parse(std::io::Cursor::new(many)).await.is_err());
    }
}
