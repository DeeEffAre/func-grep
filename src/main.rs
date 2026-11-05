use anyhow::Result;
use clap::Parser;
use func_grep::{App, Cli};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let app = App::new(&cli.path)?;
    app.process_files(&cli)
}
