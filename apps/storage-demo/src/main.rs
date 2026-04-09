/// VyomaOS persistent storage demo
///
/// Writes a counter file to /data, reads it back, increments it, and writes
/// it again. On subsequent boots the counter persists across reboots, proving
/// that /data is backed by the virtio-blk ext4 disk rather than tmpfs.
fn main() {
    let counter_path = "/data/boot_count.txt";

    // Read existing counter, or start at 0
    let count: u64 = std::fs::read_to_string(counter_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let next = count + 1;
    eprintln!("storage-demo: boot count = {next} (was {count})");

    // Write updated counter
    std::fs::write(counter_path, next.to_string()).expect("write /data/boot_count.txt");
    eprintln!("storage-demo: wrote boot_count.txt");

    // Write a timestamped log entry (appended, not replaced)
    let log_path = "/data/boot_log.txt";
    let entry = format!("boot #{next}\n");
    let existing = std::fs::read_to_string(log_path).unwrap_or_default();
    std::fs::write(log_path, format!("{existing}{entry}")).expect("write /data/boot_log.txt");

    // Read back and print all log entries
    let log = std::fs::read_to_string(log_path).expect("read /data/boot_log.txt");
    eprintln!("storage-demo: boot_log.txt contents:");
    for line in log.lines() {
        eprintln!("  {line}");
    }

    // List all files in /data
    eprintln!("storage-demo: /data directory:");
    match std::fs::read_dir("/data") {
        Ok(entries) => {
            for entry in entries.flatten() {
                let meta = entry.metadata().ok();
                let size = meta.map(|m| m.len()).unwrap_or(0);
                eprintln!("  {} ({} bytes)", entry.file_name().to_string_lossy(), size);
            }
        }
        Err(e) => eprintln!("  (error reading /data: {e})"),
    }
}
