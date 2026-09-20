use clap::Parser;
use server_core::{cli::Cli, config::ServerConfig};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = ServerConfig::read_from_path(&cli.config_path)?;

    println!("{config:?}");

    Ok(())
}
