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
    println!(
        "{}",
        serde_json::to_string(value).expect("failed to serialize JSON")
    );
}

/// # Panics
/// Panics if the error envelope cannot be serialized as JSON.
pub fn print_error(error: &CliError, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            eprintln!(
                "{}",
                serde_json::to_string(&error.to_json()).expect("failed to serialize JSON")
            );
        }
        OutputFormat::Human => {
            eprintln!("Error: {error}");
        }
    }
}

pub fn print_not_implemented(command: &str, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            print_json(&serde_json::json!({
                "error": true,
                "code": "not_implemented",
                "message": format!("Command '{command}' is not yet implemented"),
            }));
        }
        OutputFormat::Human => {
            println!("Command '{command}' is not yet implemented.");
        }
    }
}

pub fn print_device_info(info: &DeviceInfo, format: OutputFormat) {
    match format {
        OutputFormat::Json => print_json(info),
        OutputFormat::Human => {
            println!("host:             {}", info.host);
            println!("connection_type:  {}", info.connection_type);
            println!("firmware_version: {}", info.firmware_version);
            println!("battery_percent:  {}%", info.battery_percent);
            println!("disk_total_mb:    {}", info.disk_total_mb);
            println!("disk_used_mb:     {}", info.disk_used_mb);
            println!("disk_free_mb:     {}", info.disk_free_mb);
        }
    }
}

pub fn log_verbose(global: &GlobalOptions, msg: &str) {
    if global.verbose && !global.quiet {
        eprintln!("{msg}");
    }
}
