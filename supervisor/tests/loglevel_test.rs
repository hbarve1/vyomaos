// Unit tests for parse_log_level and LogLevel ordering.

#[test]
fn parse_info() { assert_eq!(supervisor::ipc::parse_log_level("info"), Some(supervisor::ipc::LogLevel::Info)); }
#[test]
fn parse_warn() { assert_eq!(supervisor::ipc::parse_log_level("warn"), Some(supervisor::ipc::LogLevel::Warn)); }
#[test]
fn parse_error() { assert_eq!(supervisor::ipc::parse_log_level("error"), Some(supervisor::ipc::LogLevel::Error)); }
#[test]
fn parse_debug() { assert_eq!(supervisor::ipc::parse_log_level("debug"), Some(supervisor::ipc::LogLevel::Debug)); }
#[test]
fn parse_upper() { assert_eq!(supervisor::ipc::parse_log_level("INFO"), Some(supervisor::ipc::LogLevel::Info)); }
#[test]
fn parse_mixed() { assert_eq!(supervisor::ipc::parse_log_level("Warn"), Some(supervisor::ipc::LogLevel::Warn)); }
#[test]
fn parse_invalid() { assert_eq!(supervisor::ipc::parse_log_level("verbose"), None); }
#[test]
fn parse_empty() { assert_eq!(supervisor::ipc::parse_log_level(""), None); }
#[test]
fn log_levels_ordered() {
    assert!(supervisor::ipc::LogLevel::Debug < supervisor::ipc::LogLevel::Info);
    assert!(supervisor::ipc::LogLevel::Info  < supervisor::ipc::LogLevel::Warn);
    assert!(supervisor::ipc::LogLevel::Warn  < supervisor::ipc::LogLevel::Error);
}
