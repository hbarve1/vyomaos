// Unit tests for session serialize/deserialize round-trip.

// The session module lives in the binary crate, but serialize/deserialize
// are re-tested here via the public supervisor lib if exposed, or we
// duplicate the pure logic inline for integration-level coverage.

/// Minimal session entry mirroring the binary crate's SessionEntry.
#[derive(Debug, Clone, PartialEq)]
struct SessionEntry {
    app_name: String,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    workspace: usize,
    focused: bool,
}

fn serialize(entries: &[SessionEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str("[[app]]\n");
        out.push_str(&format!("name = \"{}\"\n", e.app_name));
        out.push_str(&format!("x = {}\n", e.x));
        out.push_str(&format!("y = {}\n", e.y));
        out.push_str(&format!("w = {}\n", e.w));
        out.push_str(&format!("h = {}\n", e.h));
        out.push_str(&format!("workspace = {}\n", e.workspace));
        out.push_str(&format!("focused = {}\n", e.focused));
        out.push('\n');
    }
    out
}

fn deserialize(content: &str) -> Vec<SessionEntry> {
    let mut entries = Vec::new();
    let mut current: Option<SessionEntry> = None;
    for line in content.lines() {
        let l = line.trim();
        if l == "[[app]]" {
            if let Some(entry) = current.take() {
                if !entry.app_name.is_empty() {
                    entries.push(entry);
                }
            }
            current = Some(SessionEntry {
                app_name: String::new(),
                x: 0, y: 0, w: 0, h: 0,
                workspace: 0, focused: false,
            });
        } else if let Some(ref mut entry) = current {
            if let Some(v) = l.strip_prefix("name = ") {
                entry.app_name = v.trim_matches('"').to_string();
            } else if let Some(v) = l.strip_prefix("x = ") {
                entry.x = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("y = ") {
                entry.y = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("w = ") {
                entry.w = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("h = ") {
                entry.h = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("workspace = ") {
                entry.workspace = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("focused = ") {
                entry.focused = v == "true";
            }
        }
    }
    if let Some(entry) = current {
        if !entry.app_name.is_empty() {
            entries.push(entry);
        }
    }
    entries
}

#[test]
fn round_trip_empty() {
    let toml = serialize(&[]);
    let parsed = deserialize(&toml);
    assert!(parsed.is_empty());
}

#[test]
fn round_trip_single() {
    let original = vec![SessionEntry {
        app_name: "shell".into(),
        x: 0, y: 24, w: 640, h: 480,
        workspace: 0, focused: true,
    }];
    let toml = serialize(&original);
    let parsed = deserialize(&toml);
    assert_eq!(original, parsed);
}

#[test]
fn round_trip_multiple() {
    let original = vec![
        SessionEntry {
            app_name: "calc".into(),
            x: 100, y: 200, w: 320, h: 240,
            workspace: 0, focused: false,
        },
        SessionEntry {
            app_name: "editor".into(),
            x: 50, y: 50, w: 800, h: 600,
            workspace: 2, focused: true,
        },
    ];
    let toml = serialize(&original);
    let parsed = deserialize(&toml);
    assert_eq!(original, parsed);
}

#[test]
fn deserialize_handles_missing_optional_fields() {
    let input = "[[app]]\nname = \"foo\"\nw = 100\nh = 200\n";
    let entries = deserialize(input);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].x, 0);
    assert_eq!(entries[0].y, 0);
    assert_eq!(entries[0].workspace, 0);
    assert!(!entries[0].focused);
}

#[test]
fn deserialize_skips_nameless_entries() {
    let input = "[[app]]\nx = 10\ny = 20\nw = 100\nh = 100\n";
    let entries = deserialize(input);
    assert!(entries.is_empty());
}

#[test]
fn deserialize_ignores_unknown_keys() {
    let input = "[[app]]\nname = \"bar\"\nx = 1\ny = 2\nw = 3\nh = 4\nworkspace = 0\nfocused = false\nextra = 99\n";
    let entries = deserialize(input);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].app_name, "bar");
}

#[test]
fn serialize_format_contains_expected_toml_structure() {
    let entries = vec![SessionEntry {
        app_name: "test-app".into(),
        x: 10, y: 20, w: 300, h: 400,
        workspace: 1, focused: true,
    }];
    let toml = serialize(&entries);
    assert!(toml.contains("[[app]]"));
    assert!(toml.contains("name = \"test-app\""));
    assert!(toml.contains("workspace = 1"));
    assert!(toml.contains("focused = true"));
}
