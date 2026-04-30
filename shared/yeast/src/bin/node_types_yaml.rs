use clap::Parser;
use std::io::Read;

#[derive(Parser)]
#[clap(
    name = "node-types-yaml",
    about = "Convert a YAML node-types file to tree-sitter node-types.json"
)]
struct Cli {
    /// Input YAML file (reads from stdin if not provided)
    input: Option<String>,
}

fn main() {
    let args = Cli::parse();

    let yaml = match &args.input {
        Some(path) => std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("Error reading {path}: {e}");
            std::process::exit(1);
        }),
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf).unwrap_or_else(|e| {
                eprintln!("Error reading stdin: {e}");
                std::process::exit(1);
            });
            buf
        }
    };

    match yeast::node_types_yaml::convert(&yaml) {
        Ok(json) => println!("{json}"),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
