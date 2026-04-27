use std::process;

#[tokio::main]
async fn main() {
    process::exit(remarkable_tablet_cli_rs::app::run().await);
}
