use anyhow::Result;

fn main() -> Result<()> {
    xwin_screenshot::cli::run_from_env()
}
