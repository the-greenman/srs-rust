mod commands;
mod output;
pub mod payload;

use clap::Parser;
use commands::{Cli, OutputFormat};
use output::OutputDTO;
use std::io::IsTerminal;
use std::process;

fn main() {
    let cli = Cli::parse();
    let format = cli.format;
    let pretty = cli.pretty || std::io::stdout().is_terminal();

    match commands::dispatch(cli) {
        Ok(result) => {
            // result is already a rendered string from the command handler
            // For now, handlers use output::ok/err which render compact JSON
            // In the future, handlers will return OutputDTO and render here
            if format == OutputFormat::Text {
                // If text format requested but command returned JSON,
                // return a diagnostic envelope
                let dto = OutputDTO::err(
                    "srs",
                    vec!["Text format is planned but not yet implemented".to_string()],
                );
                println!("{}", dto.render(format, pretty));
            } else {
                // Parse the JSON result and re-render with pretty if needed
                let dto: OutputDTO = serde_json::from_str(&result)
                    .unwrap_or_else(|_| OutputDTO::err("srs", vec![result]));
                println!("{}", dto.render(format, pretty));
            }
            process::exit(0);
        }
        Err(e) => {
            let dto = OutputDTO::err("srs", vec![e.to_string()]);
            println!("{}", dto.render(format, pretty));
            process::exit(1);
        }
    }
}
