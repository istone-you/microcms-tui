mod app;
mod config;
mod microcms;
mod tui;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;

use crate::config::{ConfigOverrides, FileConfig};

#[derive(clap::Parser)]
struct Args {
    #[arg(long)]
    service_id: Option<String>,
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    endpoint: Option<String>,
    #[arg(long)]
    save_config: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let file = config::load_file_config()?;
    let env = ConfigOverrides {
        service_id: read_env("MICROCMS_SERVICE_ID")?,
        api_key: read_env("MICROCMS_API_KEY")?,
        endpoint: None,
    };
    let cli = ConfigOverrides {
        service_id: args.service_id,
        api_key: args.api_key,
        endpoint: args.endpoint,
    };
    let config = config::effective_config(file, env, cli);

    if args.save_config {
        config::save_file_config(&FileConfig {
            service_id: config.service_id.clone(),
            api_key: config.api_key.clone(),
            default_endpoint: config.endpoint.clone(),
        })?;
    }

    tui::run(config)
}

fn read_env(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {name}")),
    }
}
