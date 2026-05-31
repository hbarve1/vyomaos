// Unit tests for P52 file association helpers (extension extraction, format).

#[test]
fn extract_extension_txt() {
    assert_eq!(
        supervisor::file_assoc::extract_extension("readme.txt"),
        Some("txt".to_string()),
    );
}

#[test]
fn extract_extension_uppercase() {
    assert_eq!(
        supervisor::file_assoc::extract_extension("/data/Photo.PNG"),
        Some("png".to_string()),
    );
}

#[test]
fn extract_extension_nested() {
    assert_eq!(
        supervisor::file_assoc::extract_extension("archive.tar.gz"),
        Some("gz".to_string()),
    );
}

#[test]
fn extract_extension_none_for_no_dot() {
    assert_eq!(
        supervisor::file_assoc::extract_extension("Makefile"),
        None,
    );
}

#[test]
fn format_open_message_basic() {
    assert_eq!(
        supervisor::file_assoc::format_open_message("/data/notes.txt"),
        "VYOMA_SYSTEM:open:/data/notes.txt",
    );
}

#[test]
fn app_for_file_txt() {
    let app = supervisor::file_assoc::app_for_file("/data/hello.txt");
    assert_eq!(app, Some("text-editor".to_string()));
}

#[test]
fn app_for_file_png() {
    let app = supervisor::file_assoc::app_for_file("/data/photo.png");
    assert_eq!(app, Some("image-viewer".to_string()));
}

#[test]
fn app_for_file_wasm() {
    let app = supervisor::file_assoc::app_for_file("/data/app.wasm");
    assert_eq!(app, Some("app-store".to_string()));
}

#[test]
fn app_for_file_unknown() {
    let app = supervisor::file_assoc::app_for_file("/data/archive.zip");
    assert_eq!(app, None);
}

#[test]
fn list_associations_not_empty() {
    let assocs = supervisor::file_assoc::list_associations();
    assert!(!assocs.is_empty());
    // Should be sorted by extension.
    let exts: Vec<&str> = assocs.iter().map(|a| a.extension.as_str()).collect();
    let mut sorted = exts.clone();
    sorted.sort();
    assert_eq!(exts, sorted);
}
