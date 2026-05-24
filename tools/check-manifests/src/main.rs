// check-manifests — validate all apps/*/vyoma.toml files against the manifest schema.
// Exit 0 = all valid. Exit 1 = one or more errors found.
// Output per contract: "OK   apps/<name>/vyoma.toml [<name>]" or
//                      "ERROR apps/<name>/vyoma.toml [<name>] <ErrorKind>: <message>"

use std::path::Path;
use supervisor::manifest;

fn main() {
    let apps_dir = Path::new("apps");
    if !apps_dir.exists() {
        eprintln!("ERROR: apps/ directory not found (run from repo root)");
        std::process::exit(1);
    }

    let mut entries: Vec<_> = match std::fs::read_dir(apps_dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(e) => {
            eprintln!("ERROR: cannot read apps/: {e}");
            std::process::exit(1);
        }
    };
    entries.sort_by_key(|e| e.path());

    let mut error_count = 0usize;
    let mut registered_names: Vec<String> = Vec::new();

    for entry in &entries {
        let app_dir = entry.path();
        if !app_dir.is_dir() {
            continue;
        }
        let toml_path = app_dir.join("vyoma.toml");
        if !toml_path.exists() {
            continue;
        }

        let display = toml_path.display().to_string();

        match manifest::parse_manifest(&toml_path) {
            Err(e) => {
                let kind = if e.contains("unknown field") || e.contains("unknown key") {
                    "UnknownField"
                } else if e.contains("missing field") {
                    "MissingField"
                } else {
                    "ParseError"
                };
                println!("ERROR {display} [?] {kind}: {e}");
                error_count += 1;
            }
            Ok(m) => {
                let name = &m.app.name;
                let names_slice: Vec<&str> = registered_names.iter().map(|s| s.as_str()).collect();
                match manifest::validate_manifest(&m, &names_slice) {
                    Err(e) => {
                        println!("ERROR {display} [{name}] DuplicateName: {e}");
                        error_count += 1;
                    }
                    Ok(()) => {
                        registered_names.push(name.clone());
                        println!("OK    {display} [{name}]");
                    }
                }
            }
        }
    }

    if error_count > 0 {
        eprintln!("\n{error_count} manifest error(s) found.");
        std::process::exit(1);
    }
    eprintln!("\nAll {} manifests valid.", registered_names.len());
}
