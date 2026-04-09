/// VyomaOS IPC demo — pong side
///
/// Reads messages from stdin (routed by the supervisor IPC broker),
/// and sends a reply back to the ping app for each one.
fn main() {
    use std::io::{self, BufRead};
    let stdin = io::stdin();
    let mut count = 0;

    for line in stdin.lock().lines() {
        match line {
            Ok(msg) => {
                eprintln!("pong: got message: {msg}");
                // Reply to ping via the IPC broker
                println!("@ping: pong-reply-{}", count + 1);
                count += 1;
                if count >= 3 {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    eprintln!("pong: done ({count} messages handled)");
}
