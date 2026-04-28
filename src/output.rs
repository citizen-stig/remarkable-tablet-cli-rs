use serde::{Deserialize, Serialize};

use crate::cli::GlobalOptions;
use crate::error::CliError;
use crate::tablet::DeviceInfo;

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

/// # Panics
/// Panics if `value` cannot be serialized as JSON (e.g., a non-string map key).
pub fn print_json<T: Serialize + ?Sized>(value: &T) {
    println!("{}", render_json(value));
}

/// # Panics
/// Panics if the error envelope cannot be serialized as JSON.
pub fn print_error(error: &CliError, format: OutputFormat) {
    eprintln!("{}", render_error(error, format));
}

pub fn print_device_info(info: &DeviceInfo, format: OutputFormat) {
    println!("{}", format_device_info(info, format));
}

/// # Panics
/// Panics if `info` cannot be serialized as JSON.
#[must_use]
pub fn format_device_info(info: &DeviceInfo, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => render_json(info),
        OutputFormat::Human => format!(
            "host:             {}\n\
             connection_type:  {}\n\
             firmware_version: {}\n\
             battery_percent:  {}%\n\
             disk_total_mb:    {}\n\
             disk_used_mb:     {}\n\
             disk_free_mb:     {}",
            info.host,
            info.connection_type,
            info.firmware_version,
            info.battery_percent,
            info.disk_total_mb,
            info.disk_used_mb,
            info.disk_free_mb,
        ),
    }
}

pub fn log_verbose(global: &GlobalOptions, msg: &str) {
    if global.verbose && !global.quiet {
        eprintln!("{msg}");
    }
}

/// # Panics
/// Panics if `value` cannot be serialized as JSON (e.g., a non-string map key).
#[must_use]
pub fn render_json<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value).expect("failed to serialize JSON")
}

/// # Panics
/// Panics if the error envelope cannot be serialized as JSON.
#[must_use]
pub fn render_error(error: &CliError, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => render_json(&error.to_json()),
        OutputFormat::Human => format!("Error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tablet::ConnectionType;

    fn sample_device_info() -> DeviceInfo {
        DeviceInfo {
            host: "10.11.99.1".to_string(),
            connection_type: ConnectionType::Usb,
            firmware_version: "20230412102300".to_string(),
            battery_percent: 78,
            disk_total_mb: 6144,
            disk_used_mb: 2048,
            disk_free_mb: 4096,
        }
    }

    #[test]
    fn render_json_serializes_struct() {
        #[derive(Serialize)]
        struct Sample {
            name: String,
            n: i32,
        }
        let s = render_json(&Sample {
            name: "hi".to_string(),
            n: 7,
        });
        assert_eq!(s, r#"{"name":"hi","n":7}"#);
    }

    #[test]
    fn render_error_human_prepends_error_label() {
        let err = CliError::NotFound("foo".to_string());
        assert_eq!(
            render_error(&err, OutputFormat::Human),
            "Error: Not found: foo"
        );
    }

    #[test]
    fn render_error_json_emits_envelope() {
        let err = CliError::InvalidPath("bar".to_string());
        let s = render_error(&err, OutputFormat::Json);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["error"], true);
        assert_eq!(v["code"], "invalid_path");
        assert_eq!(v["message"], "Invalid path: bar");
    }

    #[test]
    fn render_error_json_codes_match_variants() {
        for (err, code) in [
            (CliError::ConnectionFailed("x".into()), "connection_failed"),
            (CliError::AuthFailed("x".into()), "auth_failed"),
            (CliError::NotFound("x".into()), "not_found"),
            (CliError::AlreadyExists("x".into()), "already_exists"),
            (CliError::InvalidPath("x".into()), "invalid_path"),
            (CliError::PermissionDenied("x".into()), "permission_denied"),
            (CliError::XochitlError("x".into()), "xochitl_error"),
            (CliError::FormatError("x".into()), "format_error"),
            (CliError::NotImplemented("x".into()), "not_implemented"),
            (CliError::IoError("x".into()), "io_error"),
        ] {
            let v: serde_json::Value =
                serde_json::from_str(&render_error(&err, OutputFormat::Json)).unwrap();
            assert_eq!(v["code"], code, "variant {err:?} should map to {code}");
        }
    }

    #[test]
    fn format_device_info_human_lays_out_aligned_fields() {
        let s = format_device_info(&sample_device_info(), OutputFormat::Human);
        assert_eq!(
            s,
            "host:             10.11.99.1\n\
             connection_type:  usb\n\
             firmware_version: 20230412102300\n\
             battery_percent:  78%\n\
             disk_total_mb:    6144\n\
             disk_used_mb:     2048\n\
             disk_free_mb:     4096"
        );
    }

    #[test]
    fn format_device_info_json_uses_lowercase_connection_type() {
        let s = format_device_info(&sample_device_info(), OutputFormat::Json);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["host"], "10.11.99.1");
        assert_eq!(v["connection_type"], "usb");
        assert_eq!(v["firmware_version"], "20230412102300");
        assert_eq!(v["battery_percent"], 78);
        assert_eq!(v["disk_total_mb"], 6144);
        assert_eq!(v["disk_used_mb"], 2048);
        assert_eq!(v["disk_free_mb"], 4096);
    }
}
