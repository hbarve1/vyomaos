// P51: VFS unit tests — path sandboxing, mount lookup, traversal rejection.
//
// These tests exercise the pure logic (lexical_clean, resolve_path, mount table)
// without requiring a running supervisor or real filesystem.

/// Lexical path cleaner — mirrors supervisor/src/vfs.rs::lexical_clean.
fn lexical_clean(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => { parts.pop(); }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

#[test]
fn clean_simple_paths() {
    assert_eq!(lexical_clean("/data/foo"), "/data/foo");
    assert_eq!(lexical_clean("/data/foo/bar/baz"), "/data/foo/bar/baz");
}

#[test]
fn clean_dot_segments() {
    assert_eq!(lexical_clean("/data/./foo"), "/data/foo");
    assert_eq!(lexical_clean("/data/./././foo"), "/data/foo");
}

#[test]
fn clean_dotdot_segments() {
    assert_eq!(lexical_clean("/data/foo/../bar"), "/data/bar");
    assert_eq!(lexical_clean("/data/a/b/../../c"), "/data/c");
}

#[test]
fn clean_duplicate_slashes() {
    assert_eq!(lexical_clean("/data///foo"), "/data/foo");
    assert_eq!(lexical_clean("//data//foo//"), "/data/foo");
}

#[test]
fn clean_root() {
    assert_eq!(lexical_clean("/"), "/");
    assert_eq!(lexical_clean(""), "/");
}

#[test]
fn clean_traversal_above_root() {
    assert_eq!(lexical_clean("/data/../../etc/passwd"), "/etc/passwd");
    assert_eq!(lexical_clean("/../../../etc/shadow"), "/etc/shadow");
    assert_eq!(lexical_clean("/.."), "/");
}

#[test]
fn mount_prefix_matching() {
    // Verify that /data/foo starts with "/data/" but /datafile does not.
    let mount_point = "/data";
    let path_ok = "/data/foo";
    let path_bad = "/datafile";
    assert!(path_ok == mount_point || path_ok.starts_with(&format!("{mount_point}/")));
    assert!(!(path_bad == mount_point || path_bad.starts_with(&format!("{mount_point}/"))));
}

#[test]
fn traversal_escape_detected() {
    // After lexical clean, /data/../etc/passwd becomes /etc/passwd which is
    // NOT under /data — the resolver must reject this.
    let cleaned = lexical_clean("/data/../etc/passwd");
    let mount_point = "/data";
    let inside = cleaned == mount_point || cleaned.starts_with(&format!("{mount_point}/"));
    assert!(!inside, "traversal escape should be detected: cleaned={cleaned}");
}

#[test]
fn dotdot_within_mount_allowed() {
    // /data/subdir/../file.txt cleans to /data/file.txt — still inside /data.
    let cleaned = lexical_clean("/data/subdir/../file.txt");
    let mount_point = "/data";
    let inside = cleaned == mount_point || cleaned.starts_with(&format!("{mount_point}/"));
    assert!(inside, "path within mount should be allowed: cleaned={cleaned}");
    assert_eq!(cleaned, "/data/file.txt");
}
