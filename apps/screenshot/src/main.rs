mod artifact;
mod capture;
mod cli;
mod config;
mod displayd_ipc;
mod encode;
#[cfg(test)]
mod harness_displayd;
mod hotkey;
mod tray;
mod ui;

use anyhow::Result;

fn main() -> Result<()> {
    cli::run_from_env()
}
