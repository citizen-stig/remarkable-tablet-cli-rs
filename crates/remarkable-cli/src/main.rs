use std::process;

#[tokio::main]
async fn main() {
    process::exit(remarkable_cli::app::run().await);
}
