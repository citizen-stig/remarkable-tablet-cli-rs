use serde::Serialize;

use crate::error::CliError;

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum, Serialize)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

pub fn print_json(value: &impl Serialize) {
    println!("{}", serde_json::to_string(value).expect("failed to serialize JSON"));
}

pub fn print_error(error: &CliError, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            eprintln!("{}", serde_json::to_string(&error.to_json()).expect("failed to serialize JSON"));
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
