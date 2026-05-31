// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P55: Archive support — tar.gz extraction and archive listing.
//!
//! Uses `flate2` (already a transitive dep via lodepng) for gzip decompression
//! and manual tar header parsing (512-byte POSIX/ustar blocks).
//! Zip files are detected by extension but only support content listing
//! (full zip extraction requires a dedicated crate).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;

// ── Tar constants ─────────────────────────────────────────────────────────────

const TAR_BLOCK: usize = 512;
const TAR_NAME_OFF: usize = 0;
const TAR_NAME_LEN: usize = 100;
const TAR_SIZE_OFF: usize = 124;
const TAR_SIZE_LEN: usize = 12;
const TAR_TYPE_OFF: usize = 156;
const TAR_PREFIX_OFF: usize = 345;
const TAR_PREFIX_LEN: usize = 155;
const TAR_MAGIC_OFF: usize = 257;
const TAR_MAGIC_LEN: usize = 5;

// ── Tar header parsing ────────────────────────────────────────────────────────

/// File type extracted from a tar header.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TarEntryType {
    File,
    Directory,
    Symlink,
    Other,
}

/// Parsed tar header.
#[derive(Debug, Clone)]
pub struct TarHeader {
    pub path: String,
    pub size: usize,
    pub entry_type: TarEntryType,
}

/// Returns true if the 512-byte block is all zeros (end-of-archive marker).
fn is_zero_block(block: &[u8]) -> bool {
    block.iter().all(|&b| b == 0)
}

/// Extract a nul-terminated string from a fixed-width field.
fn field_str(block: &[u8], off: usize, len: usize) -> String {
    let slice = &block[off..off + len];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(len);
    String::from_utf8_lossy(&slice[..end]).to_string()
}

/// Parse an octal size field.
fn parse_octal(block: &[u8], off: usize, len: usize) -> usize {
    let s = field_str(block, off, len);
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    usize::from_str_radix(s, 8).unwrap_or(0)
}

/// Parse a single tar header block into a `TarHeader`.
pub fn parse_tar_header(block: &[u8]) -> Option<TarHeader> {
    if block.len() < TAR_BLOCK || is_zero_block(block) {
        return None;
    }

    let mut name = field_str(block, TAR_NAME_OFF, TAR_NAME_LEN);

    // ustar prefix -- prepend if present
    let magic = field_str(block, TAR_MAGIC_OFF, TAR_MAGIC_LEN);
    if magic == "ustar" {
        let prefix = field_str(block, TAR_PREFIX_OFF, TAR_PREFIX_LEN);
        if !prefix.is_empty() {
            name = format!("{prefix}/{name}");
        }
    }

    let size = parse_octal(block, TAR_SIZE_OFF, TAR_SIZE_LEN);

    let type_flag = block[TAR_TYPE_OFF];
    let entry_type = match type_flag {
        0 | b'0' => TarEntryType::File,
        b'5' => TarEntryType::Directory,
        b'2' => TarEntryType::Symlink,
        _ => TarEntryType::Other,
    };

    Some(TarHeader {
        path: name,
        size,
        entry_type,
    })
}

// ── Path sanitisation ─────────────────────────────────────────────────────────

/// Sanitise a tar entry path: strip leading `/`, reject `..` traversal.
pub fn sanitise_path(raw: &str) -> String {
    let stripped = raw.trim_start_matches('/');
    let clean: PathBuf = stripped
        .split('/')
        .filter(|c| !c.is_empty() && *c != "..")
        .collect();
    clean.to_string_lossy().to_string()
}

// ── Tar.gz extraction ─────────────────────────────────────────────────────────

/// Extract a `.tar.gz` archive to `dest_dir`.
///
/// Returns a list of extracted file paths (relative to `dest_dir`).
/// Directories are created as needed. Symlinks and other special entries
/// are skipped.
pub fn extract_tar_gz(archive_path: &str, dest_dir: &str) -> Result<Vec<String>, String> {
    let compressed = fs::read(archive_path)
        .map_err(|e| format!("cannot read {archive_path}: {e}"))?;

    let mut decoder = GzDecoder::new(&compressed[..]);
    let mut tar_data = Vec::new();
    decoder
        .read_to_end(&mut tar_data)
        .map_err(|e| format!("gzip decode failed: {e}"))?;

    let dest = Path::new(dest_dir);
    fs::create_dir_all(dest)
        .map_err(|e| format!("cannot create {dest_dir}: {e}"))?;

    let mut extracted: Vec<String> = Vec::new();
    let mut offset = 0;

    while offset + TAR_BLOCK <= tar_data.len() {
        let block = &tar_data[offset..offset + TAR_BLOCK];

        // Two consecutive zero blocks mark end of archive.
        if is_zero_block(block) {
            if offset + 2 * TAR_BLOCK <= tar_data.len()
                && is_zero_block(&tar_data[offset + TAR_BLOCK..offset + 2 * TAR_BLOCK])
            {
                break;
            }
            offset += TAR_BLOCK;
            continue;
        }

        let header = match parse_tar_header(block) {
            Some(h) => h,
            None => {
                offset += TAR_BLOCK;
                continue;
            }
        };

        offset += TAR_BLOCK; // move past header

        // Sanitise path: strip leading `/` and reject `..` components.
        let clean = sanitise_path(&header.path);
        if clean.is_empty() {
            let data_blocks = (header.size + TAR_BLOCK - 1) / TAR_BLOCK;
            offset += data_blocks * TAR_BLOCK;
            continue;
        }

        let target = dest.join(&clean);

        match header.entry_type {
            TarEntryType::Directory => {
                fs::create_dir_all(&target)
                    .map_err(|e| format!("mkdir {}: {e}", target.display()))?;
                extracted.push(clean);
            }
            TarEntryType::File => {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
                }
                let end = offset + header.size;
                if end > tar_data.len() {
                    return Err(format!(
                        "truncated archive: need {end} bytes, have {}",
                        tar_data.len()
                    ));
                }
                let content = &tar_data[offset..end];
                fs::write(&target, content)
                    .map_err(|e| format!("write {}: {e}", target.display()))?;
                extracted.push(clean);
            }
            _ => {
                // Symlinks and other types are not supported; skip data.
            }
        }

        // Advance past data blocks (rounded up to 512-byte boundary).
        let data_blocks = (header.size + TAR_BLOCK - 1) / TAR_BLOCK;
        offset += data_blocks * TAR_BLOCK;
    }

    Ok(extracted)
}

// ── Archive listing ───────────────────────────────────────────────────────────

/// List the contents of an archive file.
///
/// Supports `.tar.gz` / `.tgz` (decompresses and parses tar headers) and
/// `.zip` (reads the End-of-Central-Directory record for a file count).
pub fn list_archive(path: &str) -> Result<Vec<String>, String> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        list_tar_gz(path)
    } else if lower.ends_with(".zip") {
        list_zip_summary(path)
    } else {
        Err(format!("unsupported archive format: {path}"))
    }
}

/// List entries in a `.tar.gz` / `.tgz` archive.
fn list_tar_gz(path: &str) -> Result<Vec<String>, String> {
    let compressed =
        fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;

    let mut decoder = GzDecoder::new(&compressed[..]);
    let mut tar_data = Vec::new();
    decoder
        .read_to_end(&mut tar_data)
        .map_err(|e| format!("gzip decode failed: {e}"))?;

    let mut entries: Vec<String> = Vec::new();
    let mut offset = 0;

    while offset + TAR_BLOCK <= tar_data.len() {
        let block = &tar_data[offset..offset + TAR_BLOCK];
        if is_zero_block(block) {
            break;
        }

        if let Some(header) = parse_tar_header(block) {
            let label = match header.entry_type {
                TarEntryType::Directory => format!("{}  (dir)", header.path),
                TarEntryType::File => {
                    format!("{}  ({} bytes)", header.path, header.size)
                }
                TarEntryType::Symlink => format!("{}  (symlink)", header.path),
                TarEntryType::Other => format!("{}  (other)", header.path),
            };
            entries.push(label);

            let data_blocks = (header.size + TAR_BLOCK - 1) / TAR_BLOCK;
            offset += TAR_BLOCK + data_blocks * TAR_BLOCK;
        } else {
            offset += TAR_BLOCK;
        }
    }

    Ok(entries)
}

/// Provide a summary for a `.zip` file by scanning for the End-of-Central-Directory
/// record (last 22+ bytes). Full extraction is not implemented without a zip crate.
fn list_zip_summary(path: &str) -> Result<Vec<String>, String> {
    let data = fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;

    // Verify ZIP magic (PK\x03\x04).
    if data.len() < 4 || &data[0..4] != b"PK\x03\x04" {
        return Err(format!("{path}: not a valid ZIP file"));
    }

    // Scan backwards for EOCD signature (PK\x05\x06).
    let eocd_sig: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
    let mut file_count: Option<u16> = None;
    let search_start = if data.len() > 65557 {
        data.len() - 65557
    } else {
        0
    };
    for i in (search_start..data.len().saturating_sub(21)).rev() {
        if data[i..i + 4] == eocd_sig {
            let count = u16::from_le_bytes([data[i + 10], data[i + 11]]);
            file_count = Some(count);
            break;
        }
    }

    match file_count {
        Some(n) => Ok(vec![format!(
            "{path}: ZIP archive with {n} entries (listing requires zip crate)"
        )]),
        None => Ok(vec![format!(
            "{path}: ZIP archive (entry count unknown)"
        )]),
    }
}
