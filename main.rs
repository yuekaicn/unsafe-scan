//! unsafe-scan: count and locate `unsafe` code in a Rust project.
//!
//! Zero dependencies. Comments, string literals and char literals are blanked
//! out before scanning, so `unsafe` mentioned in docs or strings is not counted.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Kind {
    Block,
    Fn,
    Impl,
    Trait,
    Extern,
}

impl Kind {
    const ALL: [Kind; 5] = [Kind::Block, Kind::Fn, Kind::Impl, Kind::Trait, Kind::Extern];

    fn name(self) -> &'static str {
        match self {
            Kind::Block => "block",
            Kind::Fn => "fn",
            Kind::Impl => "impl",
            Kind::Trait => "trait",
            Kind::Extern => "extern",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone)]
struct Finding {
    line: usize,
    kind: Kind,
}

#[derive(Debug, Default)]
struct FileReport {
    path: PathBuf,
    lines: usize,
    findings: Vec<Finding>,
}

impl FileReport {
    fn count(&self, kind: Kind) -> usize {
        self.findings.iter().filter(|f| f.kind == kind).count()
    }
}

/// Replace the contents of comments, string literals and char literals with
/// spaces, keeping newlines so line numbers stay correct.
fn strip_non_code(src: &str) -> Vec<char> {
    let s: Vec<char> = src.chars().collect();
    let mut out = s.clone();
    let n = s.len();
    let blank = |out: &mut Vec<char>, from: usize, to: usize| {
        for c in out.iter_mut().take(to.min(n)).skip(from) {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';

    let mut i = 0;
    while i < n {
        let c = s[i];
        let next = s.get(i + 1).copied();

        // Line comment
        if c == '/' && next == Some('/') {
            let start = i;
            while i < n && s[i] != '\n' {
                i += 1;
            }
            blank(&mut out, start, i);
            continue;
        }

        // Block comment (nestable)
        if c == '/' && next == Some('*') {
            let start = i;
            let mut depth = 0;
            while i < n {
                if s[i] == '/' && s.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if s[i] == '*' && s.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            blank(&mut out, start, i);
            continue;
        }

        // Raw strings: r"..", r#".."#, br"..", cr".."
        if (c == 'r' || ((c == 'b' || c == 'c') && next == Some('r')))
            && (i == 0 || !is_ident(s[i - 1]))
        {
            let mut j = i + if c == 'r' { 1 } else { 2 };
            let mut hashes = 0;
            while j < n && s[j] == '#' {
                hashes += 1;
                j += 1;
            }
            if j < n && s[j] == '"' {
                let start = j + 1;
                let mut k = start;
                'raw: while k < n {
                    if s[k] == '"' {
                        let mut h = 0;
                        while h < hashes && k + 1 + h < n && s[k + 1 + h] == '#' {
                            h += 1;
                        }
                        if h == hashes {
                            blank(&mut out, start, k);
                            i = k + 1 + hashes;
                            break 'raw;
                        }
                    }
                    k += 1;
                }
                if k >= n {
                    blank(&mut out, start, n);
                    i = n;
                }
                continue;
            }
        }

        // Normal string (b".." and c".." handled because the prefix is just an ident char)
        if c == '"' {
            let start = i + 1;
            let mut k = start;
            while k < n && s[k] != '"' {
                if s[k] == '\\' {
                    k += 1;
                }
                k += 1;
            }
            blank(&mut out, start, k);
            i = k + 1;
            continue;
        }

        // Char literal vs lifetime
        if c == '\'' {
            if next == Some('\\') {
                let start = i + 1;
                let mut k = i + 2;
                while k < n && s[k] != '\'' && s[k] != '\n' {
                    k += 1;
                }
                blank(&mut out, start, k);
                i = k + 1;
                continue;
            }
            if s.get(i + 2) == Some(&'\'') {
                blank(&mut out, i + 1, i + 2);
                i += 3;
                continue;
            }
            // lifetime or label: leave as is
        }

        i += 1;
    }
    out
}

/// Find all `unsafe` keywords in stripped code and classify them.
fn scan_source(src: &str) -> Vec<Finding> {
    let code = strip_non_code(src);
    let n = code.len();
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';

    // Tokenize into (token, line) where token is an identifier or a single punctuation char.
    let mut tokens: Vec<(String, usize)> = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < n {
        let c = code[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if is_ident(c) {
            let start = i;
            while i < n && is_ident(code[i]) {
                i += 1;
            }
            tokens.push((code[start..i].iter().collect(), line));
        } else {
            tokens.push((c.to_string(), line));
            i += 1;
        }
    }

    let mut findings = Vec::new();
    for (idx, (tok, line)) in tokens.iter().enumerate() {
        if tok != "unsafe" {
            continue;
        }
        let next = tokens.get(idx + 1).map(|t| t.0.as_str());
        let kind = match next {
            Some("{") => Some(Kind::Block),
            Some("fn") => Some(Kind::Fn),
            Some("impl") => Some(Kind::Impl),
            Some("trait") | Some("auto") => Some(Kind::Trait),
            Some("extern") => {
                // `unsafe extern "C" fn` is a function; `unsafe extern "C" { .. }` is a block.
                let after_abi = tokens[idx + 2..]
                    .iter()
                    .map(|t| t.0.as_str())
                    .find(|t| *t != "\"");
                if after_abi == Some("fn") {
                    Some(Kind::Fn)
                } else {
                    Some(Kind::Extern)
                }
            }
            // `#[unsafe(no_mangle)]` attributes and anything else are not counted.
            _ => None,
        };
        if let Some(kind) = kind {
            findings.push(Finding { line: *line, kind });
        }
    }
    findings
}

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    if root.is_file() {
        if root.extension().is_some_and(|e| e == "rs") {
            out.push(root.to_path_buf());
        }
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name == "target" || name.starts_with('.') {
                continue;
            }
            collect_rs_files(&path, out);
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

struct Options {
    root: PathBuf,
    list: bool,
    json: bool,
    deny: bool,
}

const HELP: &str = "\
unsafe-scan — count and locate `unsafe` code in a Rust project

USAGE:
    unsafe-scan [PATH] [OPTIONS]

ARGS:
    PATH            Project directory or .rs file (default: .)

OPTIONS:
    -l, --list      Print every unsafe location (file:line kind)
        --json      Print the report as JSON
        --deny      Exit with code 1 if any unsafe code is found (for CI)
    -h, --help      Print help
    -V, --version   Print version";

fn parse_args() -> Result<Option<Options>, String> {
    let mut opts = Options {
        root: PathBuf::from("."),
        list: false,
        json: false,
        deny: false,
    };
    let mut root_set = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("unsafe-scan {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-l" | "--list" => opts.list = true,
            "--json" => opts.json = true,
            "--deny" => opts.deny = true,
            a if a.starts_with('-') => return Err(format!("unknown option: {a}")),
            a => {
                if root_set {
                    return Err("only one PATH is allowed".into());
                }
                opts.root = PathBuf::from(a);
                root_set = true;
            }
        }
    }
    Ok(Some(opts))
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(Some(o)) => o,
        Ok(None) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    if !opts.root.exists() {
        eprintln!("error: path not found: {}", opts.root.display());
        return ExitCode::from(2);
    }

    let mut files = Vec::new();
    collect_rs_files(&opts.root, &mut files);

    let mut reports = Vec::new();
    for path in files {
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        reports.push(FileReport {
            lines: src.lines().count(),
            findings: scan_source(&src),
            path,
        });
    }

    let mut totals: BTreeMap<Kind, usize> = Kind::ALL.iter().map(|k| (*k, 0)).collect();
    for r in &reports {
        for f in &r.findings {
            *totals.get_mut(&f.kind).unwrap() += 1;
        }
    }
    let total: usize = totals.values().sum();
    let total_lines: usize = reports.iter().map(|r| r.lines).sum();
    let files_with_unsafe = reports.iter().filter(|r| !r.findings.is_empty()).count();

    if opts.json {
        print_json(&reports, &totals, total, total_lines);
    } else {
        print_table(
            &reports,
            &totals,
            total,
            total_lines,
            files_with_unsafe,
            opts.list,
        );
    }

    if opts.deny && total > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn print_table(
    reports: &[FileReport],
    totals: &BTreeMap<Kind, usize>,
    total: usize,
    total_lines: usize,
    files_with_unsafe: usize,
    list: bool,
) {
    let flagged: Vec<&FileReport> = reports.iter().filter(|r| !r.findings.is_empty()).collect();
    if !flagged.is_empty() {
        let width = flagged
            .iter()
            .map(|r| r.path.display().to_string().len())
            .max()
            .unwrap_or(4)
            .max(4);
        print!("{:<width$}", "FILE");
        for k in Kind::ALL {
            print!(" {:>6}", k.name());
        }
        println!(" {:>6}", "total");
        for r in &flagged {
            print!("{:<width$}", r.path.display().to_string());
            for k in Kind::ALL {
                print!(" {:>6}", r.count(k));
            }
            println!(" {:>6}", r.findings.len());
        }
        println!();
    }

    if list {
        for r in &flagged {
            for f in &r.findings {
                println!("{}:{}  unsafe {}", r.path.display(), f.line, f.kind);
            }
        }
        if !flagged.is_empty() {
            println!();
        }
    }

    let parts: Vec<String> = totals.iter().map(|(k, v)| format!("{k}={v}")).collect();
    println!(
        "Scanned {} files ({} lines): {} unsafe item(s) in {} file(s) [{}]",
        reports.len(),
        total_lines,
        total,
        files_with_unsafe,
        parts.join(", ")
    );
}

fn print_json(
    reports: &[FileReport],
    totals: &BTreeMap<Kind, usize>,
    total: usize,
    total_lines: usize,
) {
    let mut s = String::from("{\n");
    s.push_str(&format!(
        "  \"files_scanned\": {},\n  \"lines_scanned\": {},\n  \"total\": {},\n",
        reports.len(),
        total_lines,
        total
    ));
    let t: Vec<String> = totals
        .iter()
        .map(|(k, v)| format!("\"{k}\": {v}"))
        .collect();
    s.push_str(&format!(
        "  \"by_kind\": {{{}}},\n  \"findings\": [",
        t.join(", ")
    ));
    let mut first = true;
    for r in reports {
        for f in &r.findings {
            s.push_str(if first { "\n" } else { ",\n" });
            first = false;
            s.push_str(&format!(
                "    {{\"file\": \"{}\", \"line\": {}, \"kind\": \"{}\"}}",
                json_escape(&r.path.display().to_string()),
                f.line,
                f.kind
            ));
        }
    }
    s.push_str(if first { "]\n}" } else { "\n  ]\n}" });
    println!("{s}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(usize, Kind)> {
        scan_source(src)
            .into_iter()
            .map(|f| (f.line, f.kind))
            .collect()
    }

    #[test]
    fn detects_all_kinds() {
        let src = r#"
unsafe fn a() {}
fn b() { unsafe { a() } }
unsafe impl Send for X {}
unsafe trait T {}
unsafe extern "C" { fn puts(); }
pub unsafe extern "C" fn c() {}
"#;
        assert_eq!(
            kinds(src),
            vec![
                (2, Kind::Fn),
                (3, Kind::Block),
                (4, Kind::Impl),
                (5, Kind::Trait),
                (6, Kind::Extern),
                (7, Kind::Fn),
            ]
        );
    }

    #[test]
    fn ignores_comments_and_strings() {
        let src = r##"
// unsafe { }
/* unsafe fn x() /* nested unsafe { } */ */
/// docs: unsafe impl
let s = "unsafe { }";
let r = r#"unsafe fn "quoted" "#;
let c = '{';
fn f<'a>(x: &'a str) { unsafe { } }
"##;
        assert_eq!(kinds(src), vec![(8, Kind::Block)]);
    }

    #[test]
    fn ignores_unsafe_attribute() {
        assert!(kinds("#[unsafe(no_mangle)] pub extern \"C\" fn f() {}").is_empty());
    }

    #[test]
    fn escaped_quote_in_string() {
        let src = "let s = \"a \\\" unsafe { \";\nunsafe { }";
        assert_eq!(kinds(src), vec![(2, Kind::Block)]);
    }
}
