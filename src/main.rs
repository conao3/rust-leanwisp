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
    Daemon {
        #[command(subcommand)]
        command: Daemon,
    },
}
#[derive(Subcommand)]
enum Daemon {
    /// Serve formatting requests on a Unix socket, keeping Lean processes loaded.
    StartDaemon(DaemonOptions),
    /// Stop a running daemon.
    StopDaemon(DaemonOptions),
}
#[derive(clap::Args)]
struct DaemonOptions {
    /// Socket path (default: $XDG_RUNTIME_DIR/leanwisp.sock, else /tmp/leanwisp-UID.sock).
    #[arg(long)]
    socket: Option<PathBuf>,
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
    /// Send the request to a running daemon when one is available.
    #[arg(long, overrides_with = "no_daemon")]
    daemon: bool,
    #[arg(long, overrides_with = "daemon")]
    no_daemon: bool,
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
    let formatted = match (!opts.no_daemon && opts.input.paths.is_none())
        .then(|| ask_daemon(&socket_path(None), source, width, project.as_deref()))
        .flatten()
    {
        Some(result) => result?,
        None => leanwisp::format_document_with(pool, source, width, project.as_deref())?,
    };
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

fn fnv(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
    *hash ^= 0xff;
    *hash = hash.wrapping_mul(0x100000001b3);
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
    let mut hash = 0xcbf29ce484222325u64;
    fnv(&mut hash, env!("CARGO_PKG_VERSION").as_bytes());
    fnv(
        &mut hash,
        std::env::var("LEANWISP_SYNTAX")
            .unwrap_or_default()
            .as_bytes(),
    );
    fnv(&mut hash, &width.to_le_bytes());
    fnv(&mut hash, source.as_bytes());
    let search = match project {
        Some(dir) => lean_path(dir)?,
        None => std::env::var("LEAN_PATH").unwrap_or_default(),
    };
    for module in leanwisp::imported_modules(source)? {
        let relative: PathBuf = module
            .split('.')
            .collect::<PathBuf>()
            .with_extension("olean");
        fnv(&mut hash, module.as_bytes());
        for dir in search.trim().split(':').filter(|d| !d.is_empty()) {
            let path = Path::new(dir).join(&relative);
            if let Ok(meta) = std::fs::metadata(&path) {
                fnv(&mut hash, path.as_os_str().as_encoded_bytes());
                fnv(&mut hash, &meta.len().to_le_bytes());
                let modified = meta
                    .modified()
                    .ok()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?;
                fnv(&mut hash, &modified.as_nanos().to_le_bytes());
                break;
            }
        }
    }
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
    let mut groups: std::collections::BTreeMap<Option<Vec<String>>, Vec<usize>> =
        Default::default();
    for (i, file) in files.iter().enumerate() {
        let key = std::fs::read_to_string(file)
            .ok()
            .and_then(|s| leanwisp::imported_modules(&s));
        groups.entry(key).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let reports = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..opts
            .jobs
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from))
            .clamp(1, 64)
            .min(groups.len().max(1))
        {
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

fn socket_path(socket: Option<&Path>) -> PathBuf {
    if let Some(socket) = socket {
        return socket.to_owned();
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("leanwisp.sock"),
        None => PathBuf::from(format!("/tmp/leanwisp-{}.sock", unsafe { libc::getuid() })),
    }
}

fn ask_daemon(
    socket: &Path,
    source: &str,
    width: usize,
    project: Option<&Path>,
) -> Option<Result<leanwisp::Formatted, String>> {
    use std::io::BufRead;
    let mut stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
    let request = json!({
        "Source": source,
        "Width": width,
        "Project": project,
        "Version": env!("CARGO_PKG_VERSION"),
        "Syntax": std::env::var("LEANWISP_SYNTAX").unwrap_or_default(),
    });
    writeln!(stream, "{request}").ok()?;
    let mut line = String::new();
    std::io::BufReader::new(&stream).read_line(&mut line).ok()?;
    let response: serde_json::Value = serde_json::from_str(&line).ok()?;
    if let Some(error) = response.get("Error").and_then(serde_json::Value::as_str) {
        return Some(Err(error.to_owned()));
    }
    Some(serde_json::from_value(response).map_err(|e| e.to_string()))
}

fn run_daemon(command: Daemon) -> Result<i32, String> {
    use std::io::BufRead;
    let (Daemon::StartDaemon(opts) | Daemon::StopDaemon(opts)) = &command;
    let socket = socket_path(opts.socket.as_deref());
    if let Daemon::StopDaemon(_) = command {
        let mut stream = std::os::unix::net::UnixStream::connect(&socket)
            .map_err(|e| format!("{}: {e}", socket.display()))?;
        writeln!(stream, "{}", json!({"Stop": true})).map_err(|e| e.to_string())?;
        println!("{}", json!({"Socket": socket, "Stopped": true}));
        return Ok(0);
    }
    if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        return Err(format!("{}: a daemon is already running", socket.display()));
    }
    let _ = std::fs::remove_file(&socket);
    let listener = std::os::unix::net::UnixListener::bind(&socket)
        .map_err(|e| format!("{}: {e}", socket.display()))?;
    println!("{}", json!({"Socket": socket}));
    let mut pool = leanwisp::Pool::with_capacity(4);
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut line = String::new();
        if std::io::BufReader::new(&stream)
            .read_line(&mut line)
            .is_err()
        {
            continue;
        }
        let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if request.get("Stop").is_some() {
            break;
        }
        if request["Version"] != env!("CARGO_PKG_VERSION")
            || request["Syntax"] != std::env::var("LEANWISP_SYNTAX").unwrap_or_default()
        {
            continue;
        }
        let source = request["Source"].as_str().unwrap_or_default();
        let width = request["Width"].as_u64().unwrap_or(100) as usize;
        let project = request["Project"].as_str().map(PathBuf::from);
        let response =
            match leanwisp::format_document_with(&mut pool, source, width, project.as_deref()) {
                Ok(formatted) => serde_json::to_value(formatted).unwrap_or_default(),
                Err(error) => json!({ "Error": error }),
            };
        let _ = writeln!(stream, "{response}");
    }
    let _ = std::fs::remove_file(&socket);
    Ok(0)
}

fn run(cli: Cli) -> Result<i32, String> {
    leanwisp::DEBUG.store(cli.debug, std::sync::atomic::Ordering::Relaxed);
    let command = match cli.command {
        Commands::Source { command } => command,
        Commands::Daemon { command } => return run_daemon(command),
    };
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
        format_file(&opts, path, &mut leanwisp::Pool::default())
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
