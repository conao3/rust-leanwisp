use serde::Deserialize;
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Parsed {
    tree: Value,
    document: Value,
    trivia: Vec<String>,
    tokens: Vec<[usize; 2]>,
    blocks: Vec<[usize; 2]>,
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

fn parse(source: &str, project: Option<&std::path::Path>) -> Result<Parsed, String> {
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
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start Lean parser: {e}"))?;
    // Read output concurrently with writing: large documents can exceed pipe capacity.
    let mut stdin = child.stdin.take().ok_or("missing parser stdin")?;
    let input = source.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let written = writer.join().map_err(|_| "parser writer panicked")?;
    if !output.status.success() {
        return Err(format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    written.map_err(|e| e.to_string())?;
    {
        let mut de = serde_json::Deserializer::from_slice(&output.stdout);
        de.disable_recursion_limit();
        let mut parsed =
            Parsed::deserialize(&mut de).map_err(|e| format!("invalid parser response: {e}"))?;
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
        Ok(parsed)
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
            "line" | "align" => result.push((path, String::new())),
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
        if matches!(text.as_str(), "by" | "do")
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
            }
        }
        "line" => Space {
            line: !flat,
            size: usize::from(flat),
            ..Space::default()
        },
        "align" if !flat || doc[1].as_bool().unwrap_or(false) => {
            if (width as i64) < margin {
                Space {
                    size: (margin - width as i64) as usize,
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
    let mut total = 0;
    for g in groups.iter().rev() {
        for i in g.items.iter().rev() {
            let r = measure(
                i.doc,
                g.flat == Some(true),
                width as i64 + col as i64 - i.indent,
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
    let mut groups = vec![Group {
        flat: None,
        fill: false,
        items: vec![Item { indent: 0, doc }],
    }];
    let mut out = String::new();
    let mut col = 0;
    let mut pending = 0;
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
            "group" if g.flat != Some(true) => {
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
                    out.push_str(s.trim_end_matches([' ', '\t']));
                    col = 0;
                    pending = i.indent.max(0) as usize;
                    if g.flat.is_some() {
                        regroup(g, &mut groups, pending, width);
                        continue;
                    }
                } else if !s.is_empty() {
                    if s.trim() == "}" {
                        braces.pop();
                    }
                    if col == 0 && s.trim() != "}" {
                        pending = pending.max(braces.last().copied().unwrap_or(0));
                    }
                    if pending > 0 && !s.starts_with('\n') {
                        out.extend(std::iter::repeat_n(' ', pending));
                        col = pending;
                    }
                    pending = 0;
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
            "line" => {
                let mut flat = g.flat == Some(true);
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
                    out.push('\n');
                    col = 0;
                    pending = i.indent.max(0) as usize;
                    if g.fill {
                        regroup(g, &mut groups, pending, width);
                        continue;
                    }
                }
            }
            "align" if g.flat != Some(true) || d[1].as_bool().unwrap_or(false) => {
                let indent = i.indent.max(0) as usize;
                if col == 0 {
                    pending = indent;
                } else if col < indent {
                    out.extend(std::iter::repeat_n(' ', indent - col));
                    col = indent;
                } else {
                    out.push('\n');
                    col = 0;
                    pending = indent;
                }
            }
            _ => {}
        }
        groups.push(g);
    }
    out.trim_matches([' ', '\n', '\r', '\t']).to_owned() + "\n"
}

// A semicolon-separated tactic sequence cannot partly wrap while its first
// tactic remains on the `by` line. Use parser ranges to move the whole sequence.
fn layout_blocks(source: &str, parsed: &Parsed, rendered: String) -> String {
    let mut positions = Vec::new();
    let mut cursor = 0;
    for token in &parsed.tokens {
        let Some(text) = source.get(token[0]..token[1]) else {
            return rendered;
        };
        let Some(tail) = rendered.get(cursor..) else {
            return rendered;
        };
        cursor += tail.len() - tail.trim_start_matches(char::is_whitespace).len();
        if !rendered[cursor..].starts_with(text) {
            return rendered;
        }
        positions.push([cursor, cursor + text.len()]);
        cursor += text.len();
    }
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
    original: &Parsed,
    rendered: &str,
    clean: &Parsed,
) -> Result<String, String> {
    if original.tree != clean.tree || original.tokens.len() != clean.tokens.len() {
        return Err("formatting would change the Lean syntax tree; input unchanged".into());
    }
    let mut edits = Vec::new();
    let mut old_end = 0;
    let mut new_end = 0;
    for (old, new) in original.tokens.iter().zip(&clean.tokens) {
        let gap = source
            .get(old_end..old[0])
            .ok_or("overlapping syntax token ranges")?;
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
    let mut result = rendered.to_owned();
    for (start, end, gap) in edits.into_iter().rev() {
        result.replace_range(start..end, &gap);
    }
    Ok(result.trim_end_matches([' ', '\n', '\r', '\t']).to_owned() + "\n")
}

pub fn format_source(source: &str, width: usize) -> Result<String, String> {
    format_source_in(source, width, None)
}

pub fn format_source_in(
    source: &str,
    width: usize,
    project: Option<&std::path::Path>,
) -> Result<String, String> {
    if !(20..=300).contains(&width) {
        return Err("line width must be between 20 and 300".into());
    }
    let original = parse(source, project)?;
    let formatted = layout_blocks(source, &original, render(&original.document, width));
    let clean = parse(&formatted, project)?;
    let formatted = restore_comments(source, &original, &formatted, &clean)?;
    let checked = parse(&formatted, project)?;
    if original.tree != checked.tree {
        return Err("formatting would change the Lean syntax tree; input unchanged".into());
    }
    if comments(&original.trivia) != comments(&checked.trivia) {
        return Err("formatting would change comment contents or ordering; input unchanged".into());
    }
    // A formatter must settle after one pass, including its handling of comments.
    let next = layout_blocks(&formatted, &checked, render(&checked.document, width));
    let next_clean = parse(&next, project)?;
    if restore_comments(&formatted, &checked, &next, &next_clean)? != formatted {
        return Err("formatter did not reach a stable layout; input unchanged".into());
    }
    Ok(formatted)
}
