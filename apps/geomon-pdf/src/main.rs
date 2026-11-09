use geomon_pdf::{Error, Result};
use std::{collections::BTreeMap, path::Path};

fn usage() -> Error {
    Error::Invalid("Use inspect INPUT [--max-pages 100], render INPUT --page INDEX [--region X0 Y0 X1 Y1] --output DIR [--dpi 300] [--max-pixels 100000000], verify-markup BASE EDITED, or extract INPUT --pages 0,1 --output FILE.".into())
}

fn number<T: std::str::FromStr>(options: &BTreeMap<String, Vec<String>>, name: &str, default: &str) -> Result<T> {
    options.get(name).and_then(|values| values.first()).map(String::as_str).unwrap_or(default).parse().map_err(|_| usage())
}

fn run() -> Result<serde_json::Value> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;
    let source = args.next().ok_or_else(usage)?;
    let mut positional = Vec::new();
    let mut options = BTreeMap::<String, Vec<String>>::new();
    while let Some(arg) = args.next() {
        if !arg.starts_with("--") {
            positional.push(arg);
            continue;
        }
        if !["--page", "--region", "--dpi", "--max-pixels", "--max-pages", "--output", "--pages"].contains(&arg.as_str())
            || options.contains_key(&arg)
        {
            return Err(usage());
        }
        let count = if arg == "--region" { 4 } else { 1 };
        let values: Vec<String> = args.by_ref().take(count).collect();
        if values.len() != count {
            return Err(usage());
        }
        options.insert(arg, values);
    }
    let max_pages = number(&options, "--max-pages", "100")?;
    let output = || options.get("--output").and_then(|values| values.first()).map(Path::new).ok_or_else(usage);
    match command.as_str() {
        "inspect" if positional.is_empty() => Ok(serde_json::to_value(geomon_pdf::inspect(Path::new(&source), max_pages)?)?),
        "render" if positional.is_empty() => {
            let region = options
                .get("--region")
                .map(|values| {
                    let parsed: Vec<f64> = values.iter().map(|value| value.parse().map_err(|_| usage())).collect::<Result<_>>()?;
                    parsed.try_into().map_err(|_| usage())
                })
                .transpose()?;
            Ok(serde_json::to_value(geomon_pdf::render(
                Path::new(&source),
                number(&options, "--page", "0")?,
                region,
                number(&options, "--dpi", "300")?,
                number(&options, "--max-pixels", "100000000")?,
                output()?,
                max_pages,
            )?)?)
        }
        "verify-markup" if positional.len() == 1 => {
            let edited = positional.first().ok_or_else(usage)?;
            Ok(serde_json::to_value(geomon_pdf::verify_markup(Path::new(&source), Path::new(edited), max_pages)?)?)
        }
        "extract" if positional.is_empty() => {
            let selected = options.get("--pages").and_then(|values| values.first()).ok_or_else(usage)?;
            let pages: Vec<usize> = selected.split(',').map(|page| page.parse().map_err(|_| usage())).collect::<Result<_>>()?;
            Ok(serde_json::to_value(geomon_pdf::extract(Path::new(&source), &pages, output()?, max_pages)?)?)
        }
        _ => Err(usage()),
    }
}

fn main() {
    match std::panic::catch_unwind(run) {
        Ok(Ok(value)) => println!("{value}"),
        result => {
            let message = match result {
                Ok(Err(error)) => error.to_string(),
                _ => "The PDF processor could not safely read this drawing. Export a static PDF and upload it again.".into(),
            };
            eprintln!("{}", serde_json::json!({"error": message}));
            std::process::exit(2);
        }
    }
}
