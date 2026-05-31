// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Unit tests for P55 archive support: tar header parsing, path sanitisation,
//! and tar.gz extraction round-trip.

use supervisor::archive::{extract_tar_gz, list_archive, parse_tar_header, TarEntryType};

// ── Tar header constants (mirrored from archive module) ───────────────────────

const TAR_BLOCK: usize = 512;
const TAR_NAME_LEN: usize = 100;
const TAR_SIZE_OFF: usize = 124;
const TAR_TYPE_OFF: usize = 156;
const TAR_MAGIC_OFF: usize = 257;
const TAR_PREFIX_OFF: usize = 345;

/// Build a minimal tar header block for testing.
fn make_tar_header(name: &str, size: usize, type_flag: u8) -> [u8; 512] {
    let mut block = [0u8; 512];
    let name_bytes = name.as_bytes();
    let len = name_bytes.len().min(TAR_NAME_LEN);
    block[..len].copy_from_slice(&name_bytes[..len]);
    let size_str = format!("{:011o}", size);
    block[TAR_SIZE_OFF..TAR_SIZE_OFF + size_str.len()]
        .copy_from_slice(size_str.as_bytes());
    block[TAR_TYPE_OFF] = type_flag;
    block
}

#[test]
fn parse_file_header() {
    let block = make_tar_header("hello.txt", 42, b'0');
    let h = parse_tar_header(&block).unwrap();
    assert_eq!(h.path, "hello.txt");
    assert_eq!(h.size, 42);
    assert_eq!(h.entry_type, TarEntryType::File);
}

#[test]
fn parse_directory_header() {
    let block = make_tar_header("mydir/", 0, b'5');
    let h = parse_tar_header(&block).unwrap();
    assert_eq!(h.path, "mydir/");
    assert_eq!(h.size, 0);
    assert_eq!(h.entry_type, TarEntryType::Directory);
}

#[test]
fn parse_zero_block_returns_none() {
    let block = [0u8; 512];
    assert!(parse_tar_header(&block).is_none());
}

#[test]
fn parse_null_type_flag_is_file() {
    let block = make_tar_header("data.bin", 1024, 0);
    let h = parse_tar_header(&block).unwrap();
    assert_eq!(h.entry_type, TarEntryType::File);
}

#[test]
fn parse_ustar_prefix() {
    let mut block = make_tar_header("file.txt", 10, b'0');
    let magic = b"ustar";
    block[TAR_MAGIC_OFF..TAR_MAGIC_OFF + magic.len()].copy_from_slice(magic);
    let prefix = b"long/prefix/path";
    block[TAR_PREFIX_OFF..TAR_PREFIX_OFF + prefix.len()].copy_from_slice(prefix);

    let h = parse_tar_header(&block).unwrap();
    assert_eq!(h.path, "long/prefix/path/file.txt");
}

#[test]
fn parse_symlink_header() {
    let block = make_tar_header("link.txt", 0, b'2');
    let h = parse_tar_header(&block).unwrap();
    assert_eq!(h.entry_type, TarEntryType::Symlink);
}

#[test]
fn extract_tar_gz_roundtrip() {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let dir = std::env::temp_dir().join("vyoma_archive_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Build a minimal tar archive in memory.
    let mut tar_buf: Vec<u8> = Vec::new();

    // Directory entry
    let dir_hdr = make_tar_header("testdir/", 0, b'5');
    tar_buf.extend_from_slice(&dir_hdr);

    // File entry
    let content = b"Hello, VyomaOS!\n";
    let file_hdr = make_tar_header("testdir/hello.txt", content.len(), b'0');
    tar_buf.extend_from_slice(&file_hdr);
    tar_buf.extend_from_slice(content);
    let padding = TAR_BLOCK - (content.len() % TAR_BLOCK);
    if padding < TAR_BLOCK {
        tar_buf.extend(std::iter::repeat(0u8).take(padding));
    }

    // Two zero blocks (end of archive)
    tar_buf.extend_from_slice(&[0u8; 1024]);

    // Gzip compress
    let archive_path = dir.join("test.tar.gz");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&tar_buf).unwrap();
    let compressed = encoder.finish().unwrap();
    std::fs::write(&archive_path, &compressed).unwrap();

    // Extract
    let dest = dir.join("output");
    let files = extract_tar_gz(
        archive_path.to_str().unwrap(),
        dest.to_str().unwrap(),
    )
    .unwrap();

    assert!(
        files.contains(&"testdir/".to_string())
            || files.contains(&"testdir".to_string())
    );
    assert!(files.contains(&"testdir/hello.txt".to_string()));

    // Verify content
    let extracted = std::fs::read_to_string(dest.join("testdir/hello.txt")).unwrap();
    assert_eq!(extracted, "Hello, VyomaOS!\n");

    // Cleanup
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn list_tar_gz_archive() {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let dir = std::env::temp_dir().join("vyoma_archive_list_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut tar_buf: Vec<u8> = Vec::new();
    let hdr = make_tar_header("README.md", 5, b'0');
    tar_buf.extend_from_slice(&hdr);
    tar_buf.extend_from_slice(b"hello");
    tar_buf.extend(std::iter::repeat(0u8).take(TAR_BLOCK - 5));
    tar_buf.extend_from_slice(&[0u8; 1024]);

    let archive_path = dir.join("list.tar.gz");
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&tar_buf).unwrap();
    std::fs::write(&archive_path, enc.finish().unwrap()).unwrap();

    let entries = list_archive(archive_path.to_str().unwrap()).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("README.md"));
    assert!(entries[0].contains("5 bytes"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn list_unsupported_format() {
    let result = list_archive("/tmp/foo.rar");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unsupported"));
}
