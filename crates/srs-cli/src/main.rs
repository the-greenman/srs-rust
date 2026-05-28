mod commands;
mod output;

use clap::Parser;
use commands::Cli;
use std::process;

fn main() {
    let cli = Cli::parse();

    match commands::dispatch(cli) {
        Ok(output) => {
            println!("{}", output);
            process::exit(0);
        }
        Err(e) => {
            let output = output::err("srs", vec![e.to_string()]);
            println!("{}", output);
            process::exit(1);
        }
    }
}
