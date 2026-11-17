use clap::{Parser, Subcommand, ValueEnum};
use leanwisp::policy::{self, Entry, Location, Outcome, Status};
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
    /// Files formatted in parallel with --paths (default: the number of CPUs).
    #[arg(long)]
    jobs: Option<usize>,
    /// Reuse results for unchanged inputs from the cache directory.
    #[arg(long, overrides_with = "no_cache")]
    cache: bool,
    #[arg(long, overrides_with = "cache")]
    no_cache: bool,
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
    let configured = match (opts.line_width, find_upward(start, &["leanwisp.toml"])) {
        (None, Some(dir)) => {
            let path = dir.join("leanwisp.toml");
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let config: Config =
                toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            config.line_width
        }
        _ => None,
    };
    Ok(policy::line_width(opts.line_width, configured))
}

fn format_text(
    opts: &Options,
    source: &str,
    path: Option<&Path>,
    pool: &mut leanwisp::Pool,
) -> Result<leanwisp::Formatted, String> {
    let start = path
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let project = opts
        .project_dir
        .clone()
        .or_else(|| find_upward(start, &["lakefile.toml", "lakefile.lean"]));
    let width = line_width(opts, start)?;
    let key = (!opts.no_cache)
        .then(|| cache_key(source, width, project.as_deref()))
        .flatten();
    if let Some(entry) = key.as_ref().and_then(|k| std::fs::read(k).ok())
        && let Ok(formatted) = serde_json::from_slice::<leanwisp::Formatted>(&entry)
    {
        return Ok(formatted);
    }
    let formatted = leanwisp::format_document_with(pool, source, width, project.as_deref())?;
    if let Some(key) = key
        && let Some(dir) = key.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        let temporary = key.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(
            &temporary,
            serde_json::to_vec(&formatted).unwrap_or_default(),
        )
        .is_ok()
        {
            let _ = std::fs::rename(&temporary, &key);
        }
    }
    Ok(formatted)
}

fn lean_path(dir: &Path) -> Option<String> {
    static PATHS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, String>>,
    > = std::sync::OnceLock::new();
    let paths = PATHS.get_or_init(Default::default);
    if let Some(path) = paths.lock().ok()?.get(dir) {
        return Some(path.clone());
    }
    let output = std::process::Command::new("lake")
        .args(["env", "printenv", "LEAN_PATH"])
        .current_dir(dir)
        .output()
        .ok()?;
    let path = String::from_utf8(output.stdout).ok()?;
    paths.lock().ok()?.insert(dir.to_owned(), path.clone());
    Some(path)
}

fn cache_key(source: &str, width: usize, project: Option<&Path>) -> Option<PathBuf> {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?
        .join("leanwisp");
    let search = match project {
        Some(dir) => lean_path(dir)?,
        None => std::env::var("LEAN_PATH").unwrap_or_default(),
    };
    let mut imports = Vec::new();
    for module in leanwisp::imported_modules(source)? {
        let relative: PathBuf = module
            .split('.')
            .collect::<PathBuf>()
            .with_extension("olean");
        let mut stamp = None;
        for dir in search.trim().split(':').filter(|d| !d.is_empty()) {
            let path = Path::new(dir).join(&relative);
            if let Ok(meta) = std::fs::metadata(&path) {
                stamp = Some(leanwisp::cache::Stamp {
                    path: path.to_string_lossy().into_owned(),
                    size: meta.len(),
                    modified: meta
                        .modified()
                        .ok()?
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()?,
                });
                break;
            }
        }
        imports.push((module, stamp));
    }
    let hash = leanwisp::cache::key(
        env!("CARGO_PKG_VERSION"),
        &std::env::var("LEANWISP_SYNTAX").unwrap_or_default(),
        width,
        source,
        imports,
    );
    Some(root.join(format!("{hash:016x}.json")))
}

fn format_file(opts: &Options, path: &Path, pool: &mut leanwisp::Pool) -> Report {
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
        let formatted = format_text(opts, &source, Some(path), pool)?;
        report.changed = source != formatted.source;
        report.kept = formatted.kept;
        report.source = formatted.source;
        if policy::writes(opts.write, report.changed) {
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
            match policy::entry(name.to_string_lossy(), kind.is_symlink(), kind.is_dir()) {
                Entry::Skip => {}
                Entry::Descend => walk(&path, out)?,
                Entry::Format => out.push(path),
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

fn schedule(files: &[PathBuf], jobs: usize) -> Vec<Vec<usize>> {
    let mut groups: std::collections::BTreeMap<Option<Vec<String>>, Vec<(usize, usize)>> =
        Default::default();
    for (i, file) in files.iter().enumerate() {
        let source = std::fs::read_to_string(file).ok();
        let size = source.as_ref().map_or(0, String::len) + 1;
        let key = source.and_then(|s| leanwisp::imported_modules(&s));
        groups.entry(key).or_default().push((i, size));
    }
    leanwisp::schedule::assign(jobs, groups.into_values().collect())
}

fn run_paths(opts: &Options, paths: &[PathBuf]) -> Result<Vec<Report>, String> {
    let files = lean_files(paths)?;
    let jobs = policy::jobs(
        opts.jobs,
        std::thread::available_parallelism().map_or(1, usize::from),
    );
    let groups = schedule(&files, jobs);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let reports = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..policy::workers(jobs, groups.len()) {
            scope.spawn(|| {
                let mut pool = leanwisp::Pool::default();
                loop {
                    let g = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(group) = groups.get(g) else { break };
                    for &i in group {
                        let report = format_file(opts, &files[i], &mut pool);
                        reports.lock().unwrap().push((i, report));
                    }
                }
            });
        }
    });
    let mut reports = reports.into_inner().unwrap();
    reports.sort_by_key(|(i, _)| *i);
    Ok(reports.into_iter().map(|(_, r)| r).collect())
}

fn outcome(report: &Report) -> Outcome {
    Outcome {
        failed: report.error.is_some(),
        changed: report.changed,
    }
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
    leanwisp::DEBUG.store(cli.debug, std::sync::atomic::Ordering::Relaxed);
    let Commands::Source { command } = cli.command;
    let check = matches!(command, Source::CheckSource(_));
    let (Source::FormatSource(opts) | Source::CheckSource(opts)) = command;
    if !policy::writable(opts.write, check, opts.input.source.is_some()) {
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
                    let status = match policy::status(outcome(r), opts.write) {
                        Status::Error => "error",
                        Status::Formatted => "formatted",
                        Status::Changed => "changed",
                        Status::Unchanged => "unchanged",
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
        return Ok(policy::exit_code(
            check,
            opts.write,
            reports.iter().map(outcome).collect(),
        ));
    }
    let report = if let Some(path) = &opts.input.file {
        format_file(&opts, path, &mut leanwisp::Pool::default())
    } else {
        let value = opts.input.source.clone().unwrap_or_default();
        let source = match policy::locate(&value) {
            Location::Stdin => {
                let mut data = String::new();
                std::io::stdin()
                    .read_to_string(&mut data)
                    .map_err(|e| e.to_string())?;
                data
            }
            Location::File { path } => std::fs::read_to_string(path).map_err(|e| e.to_string())?,
            Location::Literal { text } => text,
        };
        let formatted = format_text(&opts, &source, None, &mut leanwisp::Pool::default())?;
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
    Ok(policy::exit_code(check, opts.write, vec![outcome(&report)]))
}
fn main() {
    let cli = Cli::try_parse().unwrap_or_else(|e| {
        let success = matches!(
            e.kind(),
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
        );
        let _ = e.print();
        std::process::exit(if success { 0 } else { policy::usage_status() });
    });
    match run(cli) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(policy::failure_status());
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn schedule_splits_shared_imports_across_jobs() {
        let dir = std::env::temp_dir().join(format!("leanwisp-schedule-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut files = Vec::new();
        for (i, import) in ["A", "A", "A", "A", "B"].iter().enumerate() {
            let file = dir.join(format!("F{i}.lean"));
            std::fs::write(&file, format!("import {import}\ndef f : Nat := {i}\n")).unwrap();
            files.push(file);
        }
        let sorted = |mut groups: Vec<Vec<usize>>| {
            groups.iter_mut().for_each(|g| g.sort());
            groups.sort();
            groups
        };
        assert_eq!(
            sorted(super::schedule(&files, 1)),
            [vec![0, 1, 2, 3], vec![4]]
        );
        assert_eq!(
            sorted(super::schedule(&files, 3)),
            [vec![0, 2], vec![1, 3], vec![4]]
        );
        assert_eq!(sorted(super::schedule(&files, 8)).len(), 5);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn schedule_handles_many_files_on_a_worker_stack() {
        std::thread::spawn(|| {
            let one: Vec<Vec<(usize, usize)>> =
                vec![(0..100_000).map(|i| (i, i % 97 + 1)).collect()];
            let groups = leanwisp::schedule::assign(6, one);
            assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), 100_000);
            let many: Vec<Vec<(usize, usize)>> =
                (0..100_000).map(|i| vec![(i, i % 97 + 1)]).collect();
            assert_eq!(leanwisp::schedule::assign(6, many).len(), 100_000);
        })
        .join()
        .unwrap();
    }
}
