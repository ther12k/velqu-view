//! `velqu-css-check` — the profile checker CLI (ADR 0009).
//!
//! Reads CSS from files (or stdin when no files / `-` are given) and
//! prints one verdict per declaration and at-rule: tier, source line, and
//! the known replacement suggestion when one exists. Exits non-zero when
//! anything unsupported is found, so it can gate builds.
//!
//! ```text
//! velqu-css-check app.css tailwind.out.css
//! velqu css check < generated.css   # via stdin
//! ```

use std::io::Read;
use std::process::ExitCode;

use velqu_tailwind::{check_css, summarize};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut sources: Vec<(String, String)> = Vec::new();

    if args.is_empty() || args.iter().any(|a| a == "-") {
        let mut text = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut text) {
            eprintln!("velqu-css-check: cannot read stdin: {e}");
            return ExitCode::from(2);
        }
        sources.push(("<stdin>".to_owned(), text));
    }
    for path in args.iter().filter(|p| p.as_str() != "-") {
        match std::fs::read_to_string(path) {
            Ok(text) => sources.push((path.clone(), text)),
            Err(e) => {
                eprintln!("velqu-css-check: cannot read {path}: {e}");
                return ExitCode::from(2);
            }
        }
    }

    let mut totals = velqu_tailwind::CheckSummary::default();
    let mut unsupported = 0usize;
    for (name, text) in &sources {
        let items = check_css(text);
        if items.is_empty() {
            continue;
        }
        println!("{name}:");
        for item in &items {
            println!("  {item}");
        }
        let summary = summarize(&items);
        println!(
            "  {} checked: {} supported, {} normalized, {} unsupported",
            summary.supported + summary.normalized + summary.unsupported,
            summary.supported,
            summary.normalized,
            summary.unsupported
        );
        unsupported += summary.unsupported;
        totals.supported += summary.supported;
        totals.normalized += summary.normalized;
        totals.unsupported += summary.unsupported;
    }

    if sources.len() > 1 {
        println!(
            "total: {} supported, {} normalized, {} unsupported",
            totals.supported, totals.normalized, totals.unsupported
        );
    }
    if unsupported > 0 {
        eprintln!("velqu-css-check: {unsupported} unsupported construct(s)");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
