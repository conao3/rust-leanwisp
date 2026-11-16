use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, ValueEnum)]
enum Output {
    Json,
    Text,
    Table,
}
#[derive(Parser)]
#[command(version, about = "Format Lean 4 source with parser-checked layout")]
struct Cli {
    #[arg(long, global = true, value_enum, default_value = "json")]
    output: Output,
    #[arg(long, global = true, overrides_with = "no_debug")]
    debug: bool,
    #[arg(long, global = true, overrides_with = "debug")]
    no_debug: bool,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    Source {
        #[command(subcommand)]
        command: Source,
    },
}
#[derive(Subcommand)]
enum Source {
    /// Format one input, or every `.lean` file under --paths.
    FormatSource(Options),
    /// Report whether formatting would change the input (exit 254 if so).
    CheckSource(Options),
}
#[derive(clap::Args)]
#[group(required = true, multiple = false)]
struct Input {
    /// A UTF-8 Lean file.
    #[arg(long)]
    file: Option<PathBuf>,
    /// Literal source text, or `file://PATH` (`file:///dev/stdin` reads stdin).
    #[arg(long)]
    source: Option<String>,
    /// Files and directories; directories are searched for `.lean` files.
    #[arg(long, num_args = 1..)]
    paths: Option<Vec<PathBuf>>,
}
#[derive(clap::Args)]
struct Options {
    #[command(flatten)]
    input: Input,
    /// Write changes back to --file or --paths.
    #[arg(long, overrides_with = "no_write")]
    write: bool,
    #[arg(long, overrides_with = "write")]
    no_write: bool,
    /// Target line width (default: `line-width` in leanwisp.toml, otherwise 100).
    #[arg(long)]
    line_width: Option<usize>,
    /// Lake project to load imports from (default: found upward from the input).
    #[arg(long)]
    project_dir: Option<PathBuf>,
    /// Files formatted in parallel with --paths.
    #[arg(long, default_value_t = 4)]
    jobs: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Config {
    line_width: Option<usize>,
}

struct Report {
    path: Option<PathBuf>,
    changed: bool,
    source: String,
    kept: Vec<leanwisp::Kept>,
    error: Option<String>,
}

fn find_upward(start: &Path, names: &[&str]) -> Option<PathBuf> {
    let start = std::fs::canonicalize(start).ok()?;
    start
        .ancestors()
        .find(|p| names.iter().any(|n| p.join(n).is_file()))
        .map(Path::to_owned)
}

fn line_width(opts: &Options, start: &Path) -> Result<usize, String> {
    if let Some(width) = opts.line_width {
        return Ok(width);
    }
    let Some(dir) = find_upward(start, &["leanwisp.toml"]) else {
        return Ok(100);
    };
    let path = dir.join("leanwisp.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let config: Config = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(config.line_width.unwrap_or(100))
}

fn format_text(
    opts: &Options,
    source: &str,
    path: Option<&Path>,
) -> Result<leanwisp::Formatted, String> {
    let start = path
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let project = opts
        .project_dir
        .clone()
        .or_else(|| find_upward(start, &["lakefile.toml", "lakefile.lean"]));
    leanwisp::format_document(source, line_width(opts, start)?, project.as_deref())
}

fn format_file(opts: &Options, path: &Path) -> Report {
    let mut report = Report {
        path: Some(path.to_owned()),
        changed: false,
        source: String::new(),
        kept: Vec::new(),
        error: None,
    };
    let result = (|| {
        if path.is_symlink() {
            return Err("refusing a symlink input".to_owned());
        }
        let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let formatted = format_text(opts, &source, Some(path))?;
        report.changed = source != formatted.source;
        report.kept = formatted.kept;
        report.source = formatted.source;
        if opts.write && report.changed {
            if std::fs::read_to_string(path).map_err(|e| e.to_string())? != source {
                return Err("file changed during formatting".into());
            }
            std::fs::write(path, &report.source).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    report.error = result.err();
    report
}

fn lean_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .collect::<Result<_, _>>()
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name();
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if name.to_string_lossy().starts_with('.') || kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                walk(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "lean") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            walk(path, &mut files)?;
        } else if path.is_file() {
            files.push(path.clone());
        } else {
            return Err(format!("{}: no such file or directory", path.display()));
        }
    }
    Ok(files)
}

fn run_paths(opts: &Options, paths: &[PathBuf]) -> Result<Vec<Report>, String> {
    let files = lean_files(paths)?;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let reports = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..opts.jobs.clamp(1, 64).min(files.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(file) = files.get(i) else { break };
                    let report = format_file(opts, file);
                    reports.lock().unwrap().push((i, report));
                }
            });
        }
    });
    let mut reports = reports.into_inner().unwrap();
    reports.sort_by_key(|(i, _)| *i);
    Ok(reports.into_iter().map(|(_, r)| r).collect())
}

fn kept_mode(kept: &leanwisp::Kept) -> &'static str {
    if kept.verbatim {
        "source"
    } else {
        "line-breaks"
    }
}

fn kept_json(kept: &[leanwisp::Kept]) -> serde_json::Value {
    kept.iter()
        .map(|k| json!({"Line": k.line, "Reason": k.reason, "Kept": kept_mode(k)}))
        .collect()
}

fn warn_kept(report: &Report) {
    let name = report
        .path
        .as_ref()
        .map_or("<input>".into(), |p| p.display().to_string());
    for kept in &report.kept {
        eprintln!(
            "{name}:{}: kept {}: {}",
            kept.line,
            kept_mode(kept),
            kept.reason
        );
    }
}

fn run(cli: Cli) -> Result<i32, String> {
    let Commands::Source { command } = cli.command;
    leanwisp::DEBUG.store(cli.debug, std::sync::atomic::Ordering::Relaxed);
    let check = matches!(command, Source::CheckSource(_));
    let (Source::FormatSource(opts) | Source::CheckSource(opts)) = command;
    if opts.write && (check || opts.input.source.is_some()) {
        return Err("--write requires format-source with --file or --paths".into());
    }
    if let Some(paths) = &opts.input.paths {
        let reports = run_paths(&opts, paths)?;
        match cli.output {
            Output::Json => {
                let files: Vec<_> = reports
                    .iter()
                    .map(|r| {
                        json!({
                            "Path": r.path.as_ref().map(|p| p.display().to_string()),
                            "Changed": r.changed,
                            "Kept": kept_json(&r.kept),
                            "Error": r.error,
                        })
                    })
                    .collect();
                println!("{}", json!({ "Files": files }));
            }
            Output::Text | Output::Table => {
                if matches!(cli.output, Output::Table) {
                    println!("Status\tKept\tPath");
                }
                for r in &reports {
                    let status = match (&r.error, r.changed, opts.write) {
                        (Some(_), _, _) => "error",
                        (None, true, true) => "formatted",
                        (None, true, false) => "changed",
                        (None, false, _) => "unchanged",
                    };
                    let path = r.path.as_ref().map(|p| p.display().to_string());
                    println!("{status}\t{}\t{}", r.kept.len(), path.unwrap_or_default());
                }
            }
        }
        for r in &reports {
            warn_kept(r);
            if let Some(error) = &r.error {
                let path = r.path.as_ref().map(|p| p.display().to_string());
                eprintln!("{}: {error}", path.unwrap_or_default());
            }
        }
        let failed = reports.iter().any(|r| r.error.is_some());
        let changed = reports.iter().any(|r| r.changed && !opts.write);
        return Ok(if failed || (check && changed) { 254 } else { 0 });
    }
    let report = if let Some(path) = &opts.input.file {
        format_file(&opts, path)
    } else {
        let value = opts.input.source.clone().unwrap_or_default();
        let source = if value == "file:///dev/stdin" {
            let mut data = String::new();
            std::io::stdin()
                .read_to_string(&mut data)
                .map_err(|e| e.to_string())?;
            data
        } else if let Some(path) = value.strip_prefix("file://") {
            std::fs::read_to_string(path).map_err(|e| e.to_string())?
        } else {
            value
        };
        let formatted = format_text(&opts, &source, None)?;
        Report {
            path: None,
            changed: source != formatted.source,
            source: formatted.source,
            kept: formatted.kept,
            error: None,
        }
    };
    if let Some(error) = report.error {
        return Err(error);
    }
    warn_kept(&report);
    match cli.output {
        Output::Json => println!(
            "{}",
            json!({"Changed": report.changed, "Source": report.source, "Kept": kept_json(&report.kept)})
        ),
        Output::Text => std::io::stdout()
            .write_all(report.source.as_bytes())
            .map_err(|e| e.to_string())?,
        Output::Table => println!("Changed\tKept\n{}\t{}", report.changed, report.kept.len()),
    }
    Ok(if check && report.changed { 254 } else { 0 })
}
fn main() {
    let cli = Cli::try_parse().unwrap_or_else(|e| {
        let success = matches!(
            e.kind(),
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
        );
        let _ = e.print();
        std::process::exit(if success { 0 } else { 252 });
    });
    match run(cli) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(254);
        }
    }
}
