// Unit tests for supervisor/src/packages.rs — package catalog and install tracking.
//
// The actual install/remove functions do filesystem I/O to /data/.
// We test the pure data aspects: catalog constants, installed.txt parsing logic.

// ── Package catalog ─────────────────────────────────────────────────────────

/// Verify the PACKAGES constant compiles and has expected structure.
/// We can't import from the binary crate, so replicate the constant here.
const PACKAGES: &[(&str, &str, &str)] = &[
    ("calculator",   "0.1.0", "Basic arithmetic calculator"),
    ("factorial",    "0.1.0", "Recursive factorial computation"),
    ("gui-demo",     "0.1.0", "Framebuffer dashboard demo"),
    ("hello-world",  "0.1.0", "Hello World demo app"),
    ("http-server",  "0.1.0", "HTTP status server on :8080"),
    ("ping",         "0.1.0", "IPC demo — send side (pair with pong)"),
    ("pong",         "0.1.0", "IPC demo — recv side (pair with ping)"),
    ("storage-demo", "0.1.0", "Persistent storage demo"),
];

#[test]
fn test_package_catalog_not_empty() {
    assert!(!PACKAGES.is_empty());
}

#[test]
fn test_package_catalog_has_calculator() {
    assert!(PACKAGES.iter().any(|&(name, _, _)| name == "calculator"));
}

#[test]
fn test_package_catalog_has_hello_world() {
    assert!(PACKAGES.iter().any(|&(name, _, _)| name == "hello-world"));
}

#[test]
fn test_package_catalog_all_have_versions() {
    for &(name, ver, _desc) in PACKAGES {
        assert!(!ver.is_empty(), "package {name} has empty version");
    }
}

#[test]
fn test_package_catalog_all_have_descriptions() {
    for &(name, _ver, desc) in PACKAGES {
        assert!(!desc.is_empty(), "package {name} has empty description");
    }
}

#[test]
fn test_package_catalog_unique_names() {
    let mut names: Vec<&str> = PACKAGES.iter().map(|&(n, _, _)| n).collect();
    let original_len = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), original_len, "duplicate package names found");
}

// ── installed.txt parsing logic (replicate read_installed_apps) ─────────────

fn parse_installed_apps(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

#[test]
fn test_parse_installed_empty() {
    assert!(parse_installed_apps("").is_empty());
}

#[test]
fn test_parse_installed_single() {
    let apps = parse_installed_apps("calculator\n");
    assert_eq!(apps, vec!["calculator"]);
}

#[test]
fn test_parse_installed_multiple() {
    let apps = parse_installed_apps("calculator\nfactorial\nhello-world\n");
    assert_eq!(apps, vec!["calculator", "factorial", "hello-world"]);
}

#[test]
fn test_parse_installed_skips_comments() {
    let apps = parse_installed_apps("# this is a comment\ncalculator\n# another\nfactorial\n");
    assert_eq!(apps, vec!["calculator", "factorial"]);
}

#[test]
fn test_parse_installed_trims_whitespace() {
    let apps = parse_installed_apps("  calculator  \n  factorial  \n");
    assert_eq!(apps, vec!["calculator", "factorial"]);
}

#[test]
fn test_parse_installed_skips_blank_lines() {
    let apps = parse_installed_apps("calculator\n\n\nfactorial\n\n");
    assert_eq!(apps, vec!["calculator", "factorial"]);
}

// ── Remove logic: filtering installed list ──────────────────────────────────

#[test]
fn test_remove_filters_name() {
    let installed = vec!["calc".to_string(), "shell".to_string(), "monitor".to_string()];
    let kept: Vec<String> = installed.into_iter().filter(|n| n != "shell").collect();
    assert_eq!(kept, vec!["calc", "monitor"]);
}

#[test]
fn test_remove_nonexistent_no_change() {
    let installed = vec!["calc".to_string(), "shell".to_string()];
    let kept: Vec<String> = installed.into_iter().filter(|n| n != "nonexistent").collect();
    assert_eq!(kept, vec!["calc", "shell"]);
}

#[test]
fn test_remove_last_app_empty() {
    let installed = vec!["calc".to_string()];
    let kept: Vec<String> = installed.into_iter().filter(|n| n != "calc").collect();
    assert!(kept.is_empty());
}

// ── File with tempfile (filesystem round-trip) ──────────────────────────────

#[test]
fn test_installed_file_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("installed.txt");

    // Write some apps
    std::fs::write(&path, "calculator\nfactorial\n").unwrap();

    // Read them back
    let content = std::fs::read_to_string(&path).unwrap();
    let apps = parse_installed_apps(&content);
    assert_eq!(apps, vec!["calculator", "factorial"]);

    // Simulate remove
    let kept: Vec<String> = apps.into_iter().filter(|n| n != "calculator").collect();
    let new_content = if kept.is_empty() {
        String::new()
    } else {
        kept.join("\n") + "\n"
    };
    std::fs::write(&path, &new_content).unwrap();

    // Verify
    let content2 = std::fs::read_to_string(&path).unwrap();
    let apps2 = parse_installed_apps(&content2);
    assert_eq!(apps2, vec!["factorial"]);
}
