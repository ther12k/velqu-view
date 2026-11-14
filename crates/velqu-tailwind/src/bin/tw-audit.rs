//! Audit a Tailwind class vocabulary: read tokens (one per line or a
//! single line of whitespace-separated tokens) from the file argument
//! (or stdin) and report every class the compiler cannot handle.
use std::io::Read;

fn main() {
    let mut raw = String::new();
    if let Some(path) = std::env::args().nth(1) {
        raw = std::fs::read_to_string(path).expect("read vocab file");
    } else {
        std::io::stdin()
            .read_to_string(&mut raw)
            .expect("read stdin");
    }
    let classes: Vec<String> = raw
        .split_whitespace()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let refs: Vec<&str> = classes.iter().map(String::as_str).collect();
    let build = velqu_tailwind::compile_utilities(&refs);
    println!(
        "tokens: {}  compiled: {}  diagnostics: {}",
        classes.len(),
        build.rules,
        build.diagnostics.len()
    );
    for d in &build.diagnostics {
        println!("UNSUPPORTED\t{}\t{}", d.class, d.message);
    }
}
