use serde::Deserialize;
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Parsed {
    trees: Vec<String>,
    document: Value,
    trivia: Vec<String>,
    tokens: Vec<[usize; 2]>,
    blocks: Vec<[usize; 2]>,
    commands: Vec<[usize; 2]>,
    kept: Vec<(usize, usize, String)>,
    lines: Vec<usize>,
}

fn comments(trivia: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for text in trivia {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            let start = i;
            if &bytes[i..i + 2] == b"--" {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                result.push(text[start..i].trim_end_matches('\r').to_owned());
            } else if &bytes[i..i + 2] == b"/-" {
                let mut depth = 1;
                i += 2;
                while i + 1 < bytes.len() && depth > 0 {
                    if &bytes[i..i + 2] == b"/-" {
                        depth += 1;
                        i += 2;
                    } else if &bytes[i..i + 2] == b"-/" {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                result.push(text[start..i].to_owned());
            } else {
                i += 1;
            }
        }
    }
    result
}

struct Adapter {
    session: u64,
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl Adapter {
    fn start(project: Option<&std::path::Path>) -> Result<Adapter, String> {
        let mut command = if let Some(binary) = std::env::var_os("LEANWISP_SYNTAX") {
            Command::new(binary)
        } else {
            let mut cmd = Command::new("lean");
            cmd.args([
                "--run",
                concat!(env!("CARGO_MANIFEST_DIR"), "/lean/Format.lean"),
            ]);
            cmd
        };
        if let Some(dir) = project {
            let program = command.get_program().to_owned();
            let args: Vec<_> = command.get_args().map(|s| s.to_owned()).collect();
            let mut lake = Command::new("lake");
            lake.arg("env").arg(program).args(args).current_dir(dir);
            command = lake;
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // Ensure a killed editor/filter does not leave a parser process behind.
            unsafe {
                command.pre_exec(|| {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() == 1 {
                        libc::raise(libc::SIGKILL);
                    }
                    Ok(())
                });
            }
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start Lean parser: {e}"))?;
        let stdin = child.stdin.take().ok_or("missing parser stdin")?;
        let stdout = std::io::BufReader::new(child.stdout.take().ok_or("missing parser stdout")?);
        Ok(Adapter {
            session: 0,
            child,
            stdin,
            stdout,
        })
    }

    fn parse(&mut self, source: &str, keep: &Keep, document: bool) -> Result<Parsed, String> {
        use std::io::BufRead;
        let started = std::time::Instant::now();
        let request = serde_json::json!({
            "source": source,
            "verbatim": keep.verbatim,
            "lines": keep.lines,
            "document": document,
            "session": self.session,
        });
        writeln!(self.stdin, "{request}").map_err(|e| format!("Lean parser stopped: {e}"))?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("Lean parser exited".into());
        }
        let mut de = serde_json::Deserializer::from_str(&line);
        de.disable_recursion_limit();
        let response =
            Value::deserialize(&mut de).map_err(|e| format!("invalid parser response: {e}"))?;
        if let Some(error) = response.get("Error").and_then(Value::as_str) {
            return Err(error.to_owned());
        }
        let mut parsed: Parsed = serde_json::from_value(response)
            .map_err(|e| format!("invalid parser response: {e}"))?;
        let mut end = 0;
        for [start, stop] in &parsed.tokens {
            if *start < end
                || *stop < *start
                || *stop > source.len()
                || !source.is_char_boundary(*start)
                || !source.is_char_boundary(*stop)
            {
                return Err("unsupported overlapping or invalid syntax token ranges".into());
            }
            end = *stop;
        }
        attach_block_introducers(&mut parsed.document);
        trace(|| format!("parsed {} bytes in {:.2?}", source.len(), started.elapsed()));
        Ok(parsed)
    }
}

impl Drop for Adapter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// Keep `:= by` / `:= do` together in the document, so their space is included
// in width decisions instead of attaching them after the line has been rendered.
fn attach_block_introducers(doc: &mut Value) {
    fn leaves(doc: &Value, path: String, result: &mut Vec<(String, String)>) {
        match tag(doc) {
            "append" => {
                leaves(&doc[1], format!("{path}/1"), result);
                leaves(&doc[2], format!("{path}/2"), result);
            }
            "nest" => leaves(&doc[2], format!("{path}/2"), result),
            "group" => leaves(&doc[1], format!("{path}/1"), result),
            "text" => result.push((path, doc[1].as_str().unwrap_or("").to_owned())),
            "line" | "align" | "break" | "block" | "softblock" | "attrline" => {
                result.push((path, String::new()))
            }
            _ => {}
        }
    }
    let mut items = Vec::new();
    leaves(doc, String::new(), &mut items);
    let mut previous = None;
    for (i, (path, text)) in items.iter().enumerate() {
        if text.trim().is_empty() {
            continue;
        }
        let run_do = text == "Id.run"
            && items[i + 1..]
                .iter()
                .find(|(_, t)| !t.trim().is_empty())
                .is_some_and(|(_, t)| t.trim() == "do");
        if (run_do || matches!(text.as_str(), "by" | "do"))
            && let Some(j) = previous
        {
            let (prev_path, prev_text): &(String, String) = &items[j];
            if prev_text.trim() == ":=" {
                *doc.pointer_mut(prev_path).unwrap() =
                    serde_json::json!(["text", prev_text.trim_end()]);
                for (gap, _) in &items[j + 1..i] {
                    *doc.pointer_mut(gap).unwrap() = serde_json::json!(["nil"]);
                }
                *doc.pointer_mut(path).unwrap() = serde_json::json!(["text", format!(" {text}")]);
            }
        }
        previous = Some(i);
    }
}

fn push_newline(out: &mut String) {
    out.truncate(out.trim_end_matches(' ').len());
    out.push('\n');
}

#[derive(Clone, Copy)]
struct Item<'a> {
    indent: i64,
    doc: &'a Value,
}
#[derive(Clone)]
struct Group<'a> {
    flat: Option<bool>,
    fill: bool,
    items: Vec<Item<'a>>,
}
#[derive(Default)]
struct Space {
    line: bool,
    hard: bool,
    end: bool,
    size: usize,
}
fn tag(doc: &Value) -> &str {
    doc[0].as_str().unwrap_or("nil")
}
fn measure(doc: &Value, flat: bool, margin: i64, width: usize) -> Space {
    match tag(doc) {
        "text" => {
            let s = doc[1].as_str().unwrap_or("");
            let line = s.contains('\n');
            Space {
                line,
                hard: flat && line,
                size: s.split('\n').next().unwrap_or("").chars().count(),
                ..Space::default()
            }
        }
        "command" => Space {
            line: true,
            end: true,
            ..Space::default()
        },
        "line" | "softblock" | "attrline" => Space {
            line: !flat,
            size: usize::from(flat),
            ..Space::default()
        },
        "break" | "block" => Space {
            line: true,
            hard: flat && tag(doc) == "break",
            ..Space::default()
        },
        "align" if !flat || doc[1].as_bool().unwrap_or(false) => {
            if (width as i64) > margin {
                Space {
                    size: (width as i64 - margin) as usize,
                    ..Space::default()
                }
            } else {
                Space {
                    line: true,
                    ..Space::default()
                }
            }
        }
        "append" => {
            let a = measure(&doc[1], flat, margin, width);
            if a.line || a.size > width {
                a
            } else {
                let b = measure(&doc[2], flat, margin, width - a.size);
                Space {
                    size: a.size + b.size,
                    ..b
                }
            }
        }
        "nest" => measure(&doc[2], flat, margin - doc[1].as_i64().unwrap_or(0), width),
        "group" => measure(&doc[1], true, margin, width),
        _ => Space::default(),
    }
}
fn measure_groups(groups: &[Group<'_>], col: usize, mut width: usize) -> Space {
    let origin = width as i64 + col as i64;
    let mut total = 0;
    for g in groups.iter().rev() {
        for i in g.items.iter().rev() {
            let r = measure(
                i.doc,
                g.flat == Some(true) && !g.fill,
                origin - i.indent,
                width,
            );
            total += r.size;
            if r.line || r.size > width {
                return Space { size: total, ..r };
            }
            width -= r.size;
        }
    }
    Space {
        size: total,
        ..Space::default()
    }
}
fn regroup<'a>(mut g: Group<'a>, groups: &mut Vec<Group<'a>>, col: usize, width: usize) {
    g.flat = Some(!g.fill);
    let w = width.saturating_sub(col);
    let r = measure_groups(std::slice::from_ref(&g), col, w);
    let total = if r.line || r.size > w {
        r.size
    } else {
        r.size + measure_groups(groups, col, w - r.size).size
    };
    g.flat = Some(!r.hard && total <= w);
    groups.push(g);
}
/// Render Lean's all-or-none and fill groups, measuring their remaining context.
/// Literal text is kept byte-for-byte, including multiline strings and comments.
pub fn render(doc: &Value, width: usize) -> String {
    render_marked(doc, width).0
}

fn render_marked(doc: &Value, width: usize) -> (String, Vec<usize>) {
    let mut groups = vec![Group {
        flat: None,
        fill: false,
        items: vec![Item { indent: 0, doc }],
    }];
    let mut out = String::new();
    let mut col = 0;
    let mut pending = 0;
    let mut explicit = false;
    let mut marks = Vec::new();
    let mut braces = Vec::new();
    while let Some(mut g) = groups.pop() {
        let Some(i) = g.items.pop() else { continue };
        let d = i.doc;
        match tag(d) {
            "append" => {
                g.items.push(Item { doc: &d[2], ..i });
                g.items.push(Item { doc: &d[1], ..i });
            }
            "nest" => g.items.push(Item {
                indent: i.indent + d[1].as_i64().unwrap_or(0),
                doc: &d[2],
            }),
            "group" if g.flat != Some(true) || g.fill => {
                let child = Group {
                    flat: Some(false),
                    fill: d[2].as_bool().unwrap_or(false),
                    items: vec![Item { doc: &d[1], ..i }],
                };
                groups.push(g);
                regroup(child, &mut groups, col.max(pending), width);
                continue;
            }
            "group" => g.items.push(Item { doc: &d[1], ..i }),
            "text" => {
                let s = d[1].as_str().unwrap_or("");
                if s.chars().all(char::is_whitespace) && s.contains('\n') {
                    out.truncate(out.trim_end_matches(' ').len());
                    out.push_str(s.trim_end_matches([' ', '\t']));
                    col = 0;
                    pending = i.indent.max(0) as usize;
                    explicit = false;
                    if g.flat.is_some() {
                        regroup(g, &mut groups, pending, width);
                        continue;
                    }
                } else if !s.is_empty() {
                    if s.trim() == "}" {
                        braces.pop();
                    }
                    if col == 0 && s.trim() != "}" && !explicit {
                        pending = pending.max(braces.last().copied().unwrap_or(0));
                    }
                    if pending > 0 && !s.starts_with('\n') {
                        out.extend(std::iter::repeat_n(' ', pending));
                        col = pending;
                    }
                    pending = 0;
                    explicit = false;
                    out.push_str(s);
                    col = s
                        .rsplit_once('\n')
                        .map_or(col + s.chars().count(), |(_, tail)| tail.chars().count());
                    if s.trim() == "{" {
                        braces.push(col + 1);
                    }
                    if s.contains('\n') && g.flat.is_some() {
                        regroup(g, &mut groups, col, width);
                        continue;
                    }
                }
            }
            "line" | "softblock" | "attrline" => {
                let mut flat = g.flat == Some(true);
                if tag(d) == "attrline" {
                    let rest = std::iter::once(&g).chain(groups.iter().rev());
                    let mut room = width as i64 - col.max(pending) as i64 - 1;
                    flat = true;
                    'measure: for group in rest {
                        for item in group.items.iter().rev() {
                            let r = measure(item.doc, true, i64::MAX / 4, room.max(0) as usize);
                            room -= r.size as i64;
                            if r.hard || room < 0 || (r.line && !r.end) {
                                flat = false;
                                break 'measure;
                            }
                            if r.line {
                                break 'measure;
                            }
                        }
                    }
                }
                if g.fill && flat {
                    let mut trial = groups.clone();
                    regroup(
                        g.clone(),
                        &mut trial,
                        col.max(pending),
                        width.saturating_sub(1),
                    );
                    flat = trial.last().unwrap().flat == Some(true);
                    if flat {
                        g = trial.pop().unwrap();
                    }
                }
                if flat {
                    if pending > 0 {
                        out.extend(std::iter::repeat_n(' ', pending));
                        col = pending;
                        pending = 0;
                    }
                    out.push(' ');
                    col += 1;
                } else {
                    push_newline(&mut out);
                    col = 0;
                    let base = out
                        .rsplit('\n')
                        .nth(1)
                        .map_or(0, |l| l.len() - l.trim_start_matches(' ').len());
                    pending = if tag(d) == "softblock" {
                        (i.indent.max(0) as usize).min(base + 2)
                    } else {
                        i.indent.max(0) as usize
                    };
                    explicit = false;
                    if g.fill {
                        regroup(g, &mut groups, pending, width);
                        continue;
                    }
                }
            }
            "command" => marks.push(out.len()),
            "break" | "block" => {
                let line = out.rsplit('\n').next().unwrap_or("");
                let base = if col == 0 {
                    pending
                } else {
                    line.len() - line.trim_start_matches(' ').len()
                };
                push_newline(&mut out);
                col = 0;
                let delta = d[1].as_i64().unwrap_or(0);
                let desired = if tag(d) == "block" {
                    (base + 2)
                        .min(i.indent.max(0) as usize)
                        .min((base as i64 + delta).max(0) as usize)
                } else {
                    (base as i64 + d[1].as_i64().unwrap_or(0)).max(0) as usize
                };
                explicit = true;
                pending = desired;
                let offset = desired as i64 - i.indent.max(0);
                for item in &mut g.items {
                    item.indent += offset;
                }
                if g.flat.is_some() {
                    regroup(g, &mut groups, pending, width);
                    continue;
                }
            }
            "align" if g.flat != Some(true) || d[1].as_bool().unwrap_or(false) => {
                let indent = i.indent.max(0) as usize;
                if col == 0 {
                    if !explicit {
                        pending = i.indent.max(0) as usize;
                    }
                } else if col < indent {
                    out.extend(std::iter::repeat_n(' ', indent - col));
                    col = indent;
                } else {
                    push_newline(&mut out);
                    col = 0;
                    pending = i.indent.max(0) as usize;
                    explicit = false;
                }
            }
            _ => {}
        }
        groups.push(g);
    }
    let lead = out.len() - out.trim_start_matches([' ', '\n', '\r', '\t']).len();
    let marks = marks
        .into_iter()
        .map(|m: usize| m.saturating_sub(lead))
        .collect();
    (
        out.trim_matches([' ', '\n', '\r', '\t']).to_owned() + "\n",
        marks,
    )
}

// A semicolon-separated tactic sequence cannot partly wrap while its first
// tactic remains on the `by` line. Use parser ranges to move the whole sequence.
fn skip_trivia(text: &str, mut i: usize) -> usize {
    let bytes = text.as_bytes();
    loop {
        while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if bytes[i..].starts_with(b"--") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i..].starts_with(b"/-")
            && !bytes[i..].starts_with(b"/--")
            && !bytes[i..].starts_with(b"/-!")
        {
            let mut depth = 0;
            while i < bytes.len() {
                if bytes[i..].starts_with(b"/-") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"-/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else {
            return i;
        }
    }
}

fn token_positions(source: &str, parsed: &Parsed, rendered: &str) -> Option<Vec<[usize; 2]>> {
    let mut positions = Vec::new();
    let mut cursor = 0;
    for token in &parsed.tokens {
        let text = source.get(token[0]..token[1])?;
        if !rendered.get(cursor..)?.starts_with(text) {
            cursor = skip_trivia(rendered, cursor);
        }
        if !rendered.get(cursor..)?.starts_with(text) {
            return None;
        }
        positions.push([cursor, cursor + text.len()]);
        cursor += text.len();
    }
    Some(positions)
}

fn layout_blocks(source: &str, parsed: &Parsed, rendered: String) -> String {
    let Some(positions) = token_positions(source, parsed, &rendered) else {
        return rendered;
    };
    let mut edits = Vec::new();
    for block in &parsed.blocks {
        let Some(first) = parsed.tokens.iter().position(|t| t[0] == block[0]) else {
            continue;
        };
        let Some(last) = parsed.tokens.iter().rposition(|t| t[1] == block[1]) else {
            continue;
        };
        if first >= last {
            continue;
        }
        let start = positions[first][1];
        let stop = positions[last][1];
        let body = &rendered[start..stop];
        if let Some((head, tail)) = body.split_once('\n') {
            if head.trim().is_empty() {
                continue;
            }
            let indent = tail
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
                .min()
                .unwrap_or(2);
            edits.push((
                start,
                positions[first + 1][0],
                format!("\n{}", " ".repeat(indent)),
            ));
        }
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut result = rendered;
    for (start, end, text) in edits {
        result.replace_range(start..end, &text);
    }
    result
}

// Comments belong to gaps between source tokens, not formatter nesting groups.
// Preserve comment-bearing gaps, adjusting only indentation of the next token.
fn restore_comments(
    source: &str,
    original: &[[usize; 2]],
    rendered: &str,
    clean: &[[usize; 2]],
) -> String {
    restore_comments_shifted(source, original, rendered, clean, &mut [])
}

fn restore_comments_shifted(
    source: &str,
    original: &[[usize; 2]],
    rendered: &str,
    clean: &[[usize; 2]],
    offsets: &mut [usize],
) -> String {
    let mut edits = Vec::new();
    let mut old_end = 0;
    let mut new_end = 0;
    for (old, new) in original.iter().zip(clean) {
        let gap = &source[old_end..old[0]];
        if !comments(&[gap.to_owned()]).is_empty() {
            let gap = if let Some((body, _)) = gap.rsplit_once('\n') {
                let line = rendered[..new[0]].rsplit('\n').next().unwrap_or("");
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                format!("{body}\n{indent}")
            } else {
                gap.to_owned()
            };
            edits.push((new_end, new[0], gap));
        }
        old_end = old[1];
        new_end = new[1];
    }
    let gap = &source[old_end..];
    if !comments(&[gap.to_owned()]).is_empty() {
        edits.push((new_end, rendered.len(), gap.to_owned()));
    }
    for offset in offsets.iter_mut() {
        let original_offset = *offset;
        for (start, end, gap) in &edits {
            if *end <= original_offset {
                *offset = (*offset + gap.len()).saturating_sub(end - start);
            }
        }
    }
    let mut result = rendered.to_owned();
    for (start, end, gap) in edits.into_iter().rev() {
        result.replace_range(start..end, &gap);
    }
    result.trim_end_matches([' ', '\n', '\r', '\t']).to_owned() + "\n"
}

pub static DEBUG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn trace(message: impl FnOnce() -> String) {
    if DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("leanwisp: {}", message());
    }
}

#[derive(Default)]
struct Keep {
    verbatim: Vec<usize>,
    lines: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct Kept {
    pub line: usize,
    pub reason: String,
    pub verbatim: bool,
}

#[derive(Clone, Debug, serde::Serialize, Deserialize)]
pub struct Formatted {
    pub source: String,
    pub kept: Vec<Kept>,
}

enum Failure {
    Command(usize, String),
    Commands(Vec<usize>, String),
    Fatal(String),
}

fn command_trees(trees: &[String]) -> &[String] {
    trees.get(1..).unwrap_or(&[])
}

fn tree_difference(original: &Parsed, other: &Parsed, reason: &str) -> Failure {
    let (a, b) = (command_trees(&original.trees), command_trees(&other.trees));
    if a.len() == b.len() && !a.is_empty() {
        let all: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
        if !all.is_empty() {
            return Failure::Commands(all, reason.into());
        }
    }
    match (0..a.len().min(b.len())).find(|&i| a[i] != b[i]) {
        Some(i) => Failure::Command(i, reason.into()),
        None if a.len() != b.len() && !a.is_empty() && !b.is_empty() => {
            Failure::Command(a.len().min(b.len()) - 1, reason.into())
        }
        None => Failure::Fatal(format!("{reason}; input unchanged")),
    }
}

fn command_at(starts: &[usize], offset: usize) -> usize {
    starts.iter().rposition(|&s| s <= offset).unwrap_or(0)
}

fn error_offset(text: &str, message: &str) -> Option<usize> {
    let rest = message.split("input.lean:").nth(1)?;
    let mut parts = rest.splitn(3, ':');
    let line: usize = parts.next()?.parse().ok()?;
    let column: usize = parts.next()?.parse().ok()?;
    let start: usize = text
        .split_inclusive('\n')
        .take(line - 1)
        .map(str::len)
        .sum();
    let line_text = text.get(start..)?.split('\n').next()?;
    Some(
        start
            + line_text
                .chars()
                .take(column)
                .map(char::len_utf8)
                .sum::<usize>(),
    )
}

fn reparse(
    adapter: &mut Adapter,
    text: &str,
    starts: Option<Vec<usize>>,
    keep: &Keep,
    document: bool,
) -> Result<Parsed, Failure> {
    adapter.parse(text, keep, document).map_err(|message| {
        trace(|| {
            let line = error_offset(text, &message).map_or(0, |o| text[..o].matches('\n').count());
            let context: Vec<_> = text.lines().skip(line.saturating_sub(3)).take(6).collect();
            format!("reparse failed: {message}\n{}", context.join("\n"))
        });
        match (starts, error_offset(text, &message)) {
            (Some(starts), Some(offset)) => Failure::Command(
                command_at(&starts, offset),
                "formatted text does not parse".into(),
            ),
            _ => Failure::Fatal(message),
        }
    })
}

fn command_comments(text: &str, parsed: &Parsed) -> Vec<Vec<String>> {
    let starts: Vec<usize> = parsed.commands.iter().map(|c| c[0]).collect();
    (0..starts.len())
        .map(|i| {
            let start = if i == 0 { 0 } else { starts[i] };
            let stop = starts.get(i + 1).copied().unwrap_or(text.len());
            comments(&[text[start.min(stop)..stop].to_owned()])
        })
        .collect()
}

fn attempt(
    adapter: &mut Adapter,
    source: &str,
    width: usize,
    keep: &Keep,
) -> Result<(String, Parsed), Failure> {
    let original = adapter.parse(source, keep, true).map_err(Failure::Fatal)?;
    let started = std::time::Instant::now();
    let (rendered, marks) = render_marked(&original.document, width);
    trace(|| format!("rendered in {:.2?}", started.elapsed()));
    let rendered = layout_blocks(source, &original, rendered);
    let marks_rendered = marks.clone();
    let starts = Some(marks);
    let clean = match token_positions(source, &original, &rendered) {
        Some(tokens) => tokens,
        None => {
            let clean = reparse(adapter, &rendered, starts, keep, false)?;
            if original.trees != clean.trees || original.tokens.len() != clean.tokens.len() {
                return Err(tree_difference(
                    &original,
                    &clean,
                    "formatting would change the Lean syntax tree",
                ));
            }
            clean.tokens
        }
    };
    let mut starts = marks_rendered;
    let formatted =
        restore_comments_shifted(source, &original.tokens, &rendered, &clean, &mut starts);
    let starts = Some(starts);
    let checked = reparse(adapter, &formatted, starts, keep, true)?;
    if original.trees != checked.trees || original.tokens.len() != checked.tokens.len() {
        return Err(tree_difference(
            &original,
            &checked,
            "formatting would change the Lean syntax tree",
        ));
    }
    if comments(&original.trivia) != comments(&checked.trivia) {
        let (a, b) = (
            command_comments(source, &original),
            command_comments(&formatted, &checked),
        );
        return Err(match (0..a.len().min(b.len())).find(|&i| a[i] != b[i]) {
            Some(i) => Failure::Command(i, "formatting would change comments".into()),
            None => Failure::Fatal(
                "formatting would change comment contents or ordering; input unchanged".into(),
            ),
        });
    }
    // A formatter must settle after one pass, including its handling of comments.
    let (next, marks) = render_marked(&checked.document, width);
    let next = layout_blocks(&formatted, &checked, next);
    let starts = Some(marks);
    let next_clean = match token_positions(&formatted, &checked, &next) {
        Some(tokens) => tokens,
        None => {
            let next_clean = reparse(adapter, &next, starts, keep, false)?;
            if checked.trees != next_clean.trees || checked.tokens.len() != next_clean.tokens.len()
            {
                return Err(tree_difference(
                    &checked,
                    &next_clean,
                    "formatter did not reach a stable layout",
                ));
            }
            next_clean.tokens
        }
    };
    let settled = restore_comments(&formatted, &checked.tokens, &next, &next_clean);
    if settled != formatted {
        let offset = formatted
            .bytes()
            .zip(settled.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(formatted.len().min(settled.len()));
        let starts: Vec<usize> = checked.commands.iter().map(|c| c[0]).collect();
        return Err(Failure::Command(
            command_at(&starts, offset),
            "formatter did not reach a stable layout".into(),
        ));
    }
    Ok((formatted, original))
}

pub fn format_source(source: &str, width: usize) -> Result<String, String> {
    format_source_in(source, width, None)
}

pub fn format_source_in(
    source: &str,
    width: usize,
    project: Option<&std::path::Path>,
) -> Result<String, String> {
    let result = format_document(source, width, project)?;
    match result.kept.first() {
        None => Ok(result.source),
        Some(kept) => Err(format!(
            "line {}: {}; input unchanged",
            kept.line, kept.reason
        )),
    }
}

fn escalate(
    keep: &mut Keep,
    reasons: &mut std::collections::HashMap<usize, String>,
    indices: &[usize],
    reason: String,
) -> Result<(), String> {
    for &index in indices {
        trace(|| format!("command {index}: {reason}"));
        if keep.verbatim.contains(&index) {
            return Err(format!("{reason}; input unchanged"));
        }
        if keep.lines.contains(&index) {
            keep.verbatim.push(index);
        } else {
            keep.lines.push(index);
            reasons.insert(index, reason.clone());
        }
    }
    Ok(())
}

pub fn imported_modules(source: &str) -> Option<Vec<String>> {
    let mut modules = Vec::new();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("--") || line == "module" || line == "prelude" {
            continue;
        }
        if line.starts_with("/-") {
            let mut text = line.to_owned();
            while !text.contains("-/") {
                text = lines.next()?.to_owned();
            }
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(position) = words.iter().position(|w| *w == "import") else {
            break;
        };
        if !words[..position]
            .iter()
            .all(|w| matches!(*w, "public" | "meta" | "private"))
        {
            break;
        }
        modules.extend(
            words[position + 1..]
                .iter()
                .filter(|w| **w != "all")
                .map(|w| w.to_string()),
        );
    }
    Some(modules)
}

type PoolKey = (Option<std::path::PathBuf>, Vec<String>);

#[derive(Default)]
pub struct Pool {
    adapters: Vec<(PoolKey, Adapter)>,
    session: u64,
    capacity: usize,
}

impl Pool {
    pub fn with_capacity(capacity: usize) -> Pool {
        Pool {
            capacity,
            ..Pool::default()
        }
    }

    fn take(
        &mut self,
        project: Option<&std::path::Path>,
        source: &str,
    ) -> Result<(Option<PoolKey>, Adapter), String> {
        self.session += 1;
        let key = imported_modules(source).map(|m| (project.map(ToOwned::to_owned), m));
        let found = key
            .as_ref()
            .and_then(|k| self.adapters.iter().position(|(key, _)| key == k));
        let mut adapter = match found {
            Some(i) => self.adapters.remove(i).1,
            None => Adapter::start(project)?,
        };
        adapter.session = self.session;
        Ok((key, adapter))
    }

    fn put(&mut self, key: Option<PoolKey>, adapter: Adapter) {
        if let Some(key) = key {
            self.adapters.push((key, adapter));
            if self.adapters.len() > self.capacity.max(2) {
                self.adapters.remove(0);
            }
        }
    }
}

pub fn format_document(
    source: &str,
    width: usize,
    project: Option<&std::path::Path>,
) -> Result<Formatted, String> {
    format_document_with(&mut Pool::default(), source, width, project)
}

pub fn format_document_with(
    pool: &mut Pool,
    source: &str,
    width: usize,
    project: Option<&std::path::Path>,
) -> Result<Formatted, String> {
    if !(20..=300).contains(&width) {
        return Err("line width must be between 20 and 300".into());
    }
    let (key, mut adapter) = pool.take(project, source)?;
    let result = format_with(&mut adapter, source, width);
    let result = match result {
        Err(message) if message.contains("imports changed") => {
            let mut fresh = Adapter::start(project)?;
            fresh.session = pool.session;
            let result = format_with(&mut fresh, source, width);
            adapter = fresh;
            result
        }
        result => result,
    };
    if result.is_ok() {
        pool.put(key, adapter);
    }
    result
}

fn format_with(adapter: &mut Adapter, source: &str, width: usize) -> Result<Formatted, String> {
    let mut keep = Keep::default();
    let mut reasons = std::collections::HashMap::new();
    loop {
        match attempt(adapter, source, width, &keep) {
            Ok((formatted, original)) => {
                let line_of =
                    |start: usize| source[..start.min(source.len())].matches('\n').count() + 1;
                let mut kept: Vec<Kept> = original
                    .kept
                    .iter()
                    .map(|(index, start, reason)| Kept {
                        line: line_of(*start),
                        reason: reasons
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| reason.lines().next().unwrap_or("").to_owned()),
                        verbatim: true,
                    })
                    .collect();
                for index in original.lines.iter().filter(|i| !keep.verbatim.contains(i)) {
                    if let Some(range) = original.commands.get(*index) {
                        kept.push(Kept {
                            line: line_of(range[0]),
                            reason: reasons.get(index).cloned().unwrap_or_default(),
                            verbatim: false,
                        });
                    }
                }
                kept.sort_by_key(|k| k.line);
                return Ok(Formatted {
                    source: formatted,
                    kept,
                });
            }
            Err(Failure::Command(index, reason)) => {
                escalate(&mut keep, &mut reasons, &[index], reason)?
            }
            Err(Failure::Commands(indices, reason)) => {
                escalate(&mut keep, &mut reasons, &indices, reason)?
            }
            Err(Failure::Fatal(message)) => return Err(message),
        }
    }
}
