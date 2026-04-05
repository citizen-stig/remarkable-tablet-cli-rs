mod cli;
mod commands;
mod error;
mod output;

use clap::Parser;
use cli::{Cli, Command};
use std::process;

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Command::Connect => commands::connect::execute(&cli.global),
        Command::Ls(args) => commands::ls::execute(&cli.global, args),
        Command::Info(args) => commands::info::execute(&cli.global, args),
        Command::Find(args) => commands::find::execute(&cli.global, args),
        Command::Backup(args) => commands::backup::execute(&cli.global, args),
        Command::Download(args) => commands::download::execute(&cli.global, args),
        Command::Upload(args) => commands::upload::execute(&cli.global, args),
        Command::Mv(args) => commands::mv::execute(&cli.global, args),
        Command::Mkdir(args) => commands::mkdir::execute(&cli.global, args),
        Command::Rename(args) => commands::rename::execute(&cli.global, args),
        Command::Rm(args) => commands::rm::execute(&cli.global, args),
    };

    if let Err(e) = result {
        output::print_error(&e, cli.global.format);
        process::exit(e.exit_code());
    }
}
