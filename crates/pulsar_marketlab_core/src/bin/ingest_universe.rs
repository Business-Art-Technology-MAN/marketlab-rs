//! Offline ingestion binary: JerBouma/FinanceDatabase `equities.csv` → `finance_database_equities.usda`.

use std::env;
use std::fs::File;
use std::path::PathBuf;
use std::process;

use pulsar_marketlab_core::{ingest_equities_csv, FINANCE_DATABASE_EQUITIES_LAYER_FILENAME};

const USAGE: &str = "Usage: ingest_universe --input <equities.csv> [--output <finance_database_equities.usda>]\n\
\n\
Stream-parses a FinanceDatabase equities export and writes a buffered USDA mirror layer\n\
with `node_asset_{ticker}` prims under /MarketLab/Universe.";

fn parse_args() -> Result<(PathBuf, PathBuf), String> {
    let mut args = env::args().skip(1);
    let mut input = None;
    let mut output = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" | "-i" => {
                let path = args
                    .next()
                    .ok_or_else(|| "--input requires a path".to_string())?;
                input = Some(PathBuf::from(path));
            }
            "--output" | "-o" => {
                let path = args
                    .next()
                    .ok_or_else(|| "--output requires a path".to_string())?;
                output = Some(PathBuf::from(path));
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let input = input.ok_or_else(|| "--input is required".to_string())?;
    let output = output.unwrap_or_else(|| PathBuf::from(FINANCE_DATABASE_EQUITIES_LAYER_FILENAME));
    Ok((input, output))
}

fn main() {
    let (input, output) = match parse_args() {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("{err}\n\n{USAGE}");
            process::exit(2);
        }
    };

    let source = match File::open(&input) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("failed to open {}: {err}", input.display());
            process::exit(1);
        }
    };

    let destination = match File::create(&output) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("failed to create {}: {err}", output.display());
            process::exit(1);
        }
    };

    match ingest_equities_csv(source, destination) {
        Ok(count) => {
            println!(
                "ingested {count} equities into {}",
                output.display()
            );
        }
        Err(err) => {
            eprintln!("ingestion failed: {err}");
            process::exit(1);
        }
    }
}
