//! Whole-pipeline text for the aggregation Text mode and pipeline import.
//!
//! Stage bodies are kept verbatim, so a half-written stage survives a round trip.
//! Disabled stages are written as commented-out lines.

use super::aggregation::PipelineStage;

pub(crate) fn pipeline_to_text(stages: &[PipelineStage]) -> String {
    if stages.is_empty() {
        return "[]".to_string();
    }
    let mut out = String::from("[\n");
    for stage in stages {
        let mut lines = stage.body.trim().lines();
        let first = lines.next().unwrap_or("{}");
        let mut element = format!("  {{\n    {}: {first}", operator_key(stage.operator.trim()));
        for line in lines {
            element.push_str("\n    ");
            element.push_str(line);
        }
        element.push_str("\n  },");
        if stage.enabled {
            out.push_str(&element);
        } else {
            // Every element line is indented at least two spaces, like an editor's toggle-comment.
            let commented = element
                .lines()
                .map(|line| format!("  // {}", line.strip_prefix("  ").unwrap_or(line)))
                .collect::<Vec<_>>();
            out.push_str(&commented.join("\n"));
        }
        out.push('\n');
    }
    out.push(']');
    out
}

fn operator_key(operator: &str) -> String {
    let bare = !operator.is_empty()
        && operator.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '$' || ch == '_');
    if bare { operator.to_string() } else { serde_json::Value::from(operator).to_string() }
}

pub(crate) fn parse_pipeline_text(text: &str) -> Result<Vec<PipelineStage>, String> {
    let (code, disabled_lines) = uncomment_disabled_stages(text);

    let mut open = None;
    let mut close = None;
    let mut commas = Vec::new();
    let mut stray = None;
    scan(&code, |ix, byte, depth| match (byte, depth) {
        (b'[', 0) if open.is_none() => open = Some(ix),
        (b']', 1) if close.is_none() && open.is_some() => close = Some(ix),
        (b',', 1) if close.is_none() => commas.push(ix),
        (_, 0) if stray.is_none() && (open.is_none() || close.is_some()) => stray = Some(ix),
        _ => {}
    })?;
    let (Some(open), Some(close), None) = (open, close, stray) else {
        return Err("Write the pipeline as an array: [ { $match: { … } } ]".to_string());
    };

    let mut bounds = vec![open + 1];
    bounds.extend(commas.iter().map(|comma| comma + 1));
    let mut ends = commas;
    ends.push(close);

    let mut stages = Vec::new();
    for (start, end) in bounds.into_iter().zip(ends) {
        // Comments around a stage belong to no stage.
        let Some((first, last)) = code_bounds(&code[start..end]) else {
            continue;
        };
        let element = &code[start + first..=start + last];
        let number = stages.len() + 1;
        let first_line = code[..start + first].matches('\n').count();
        let last_line = first_line + element.matches('\n').count();
        let enabled = !(first_line..=last_line).all(|line| disabled_lines[line]);
        stages.push(parse_stage(element, number, enabled)?);
    }
    Ok(stages)
}

fn parse_stage(element: &str, number: usize, enabled: bool) -> Result<PipelineStage, String> {
    let shape = || format!("Stage {number}: write it as {{ $operator: … }}");
    let inner =
        element.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')).ok_or_else(shape)?;

    let mut colon = None;
    let mut commas = Vec::new();
    scan(inner, |ix, byte, depth| match (byte, depth) {
        (b':', 0) if colon.is_none() => colon = Some(ix),
        (b',', 0) => commas.push(ix),
        _ => {}
    })
    .map_err(|err| format!("Stage {number}: {err}"))?;
    let colon = colon.ok_or_else(shape)?;
    let (_, key_end) = code_bounds(&inner[..colon]).ok_or_else(shape)?;
    let key_start = code_bounds(&inner[..colon]).map_or(0, |(start, _)| start);
    let operator = inner[key_start..=key_end].trim_matches(|ch| ch == '"' || ch == '\'');
    if operator.is_empty() {
        return Err(shape());
    }

    let rest = &inner[colon + 1..];
    let Some((_, body_end)) = code_bounds(rest) else {
        return Err(format!("Stage {number}: {operator} needs a value"));
    };
    let mut body = &rest[..=body_end];
    // A single trailing comma is fine; any other top-level comma means a second operator.
    let trailing = colon + 1 + body_end;
    if commas.iter().any(|&ix| ix != trailing) {
        return Err(format!("Stage {number}: use one operator per stage"));
    }
    if commas.contains(&trailing) {
        body = &body[..body.len() - 1];
    }
    Ok(PipelineStage::with(operator.to_string(), dedent_body(body.trim()), enabled))
}

/// Byte range of the first and last code in `src`, ignoring whitespace and comments.
fn code_bounds(src: &str) -> Option<(usize, usize)> {
    let mut bounds: Option<(usize, usize)> = None;
    scan(src, |ix, _, _| {
        let first = bounds.map_or(ix, |(first, _)| first);
        bounds = Some((first, ix));
    })
    .ok()?;
    bounds
}

/// Undo the four spaces `pipeline_to_text` adds to every body line after the first.
fn dedent_body(body: &str) -> String {
    let mut lines = body.lines();
    let first = lines.next().unwrap_or_default().to_string();
    let rest: Vec<&str> = lines.collect();
    let indent = rest
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0)
        .min(4);
    std::iter::once(first)
        .chain(rest.iter().map(|line| line.get(indent..).unwrap_or(line.trim_start()).to_string()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Uncomment `//` lines that hold a whole stage between other stages, the shape of a
/// disabled stage. Other comments, including ones inside a stage, stay comments.
/// Returns the code and, per line, whether it was uncommented.
fn uncomment_disabled_stages(text: &str) -> (String, Vec<bool>) {
    fn comment_body(line: &str) -> Option<&str> {
        let rest = line.trim_start().strip_prefix("//")?;
        Some(rest.strip_prefix(' ').unwrap_or(rest))
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let array_depth = depth_at_line_starts(text, &lines);
    let mut disabled = vec![false; lines.len()];
    let mut open = 0i64;
    for (ix, line) in lines.iter().enumerate() {
        let Some(body) = comment_body(line) else {
            open = 0;
            continue;
        };
        let starts_stage = array_depth[ix] == 1 && body.trim_start().starts_with('{');
        if open > 0 || starts_stage {
            disabled[ix] = true;
            open = (open + bracket_delta(body)).max(0);
        }
    }
    let code = lines
        .iter()
        .zip(&disabled)
        .map(|(line, &flag)| match (flag, comment_body(line)) {
            (true, Some(body)) => {
                let indent = line.len() - line.trim_start().len();
                format!("{}{body}", &line[..indent])
            }
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    (code, disabled)
}

/// Bracket depth of the code before each line; comments don't count.
fn depth_at_line_starts(text: &str, lines: &[&str]) -> Vec<usize> {
    let mut changes = Vec::new();
    let _ = scan(text, |ix, byte, depth| match byte {
        b'[' | b'{' | b'(' => changes.push((ix, depth + 1)),
        b']' | b'}' | b')' => changes.push((ix, depth.saturating_sub(1))),
        _ => {}
    });
    let mut depth = 0;
    let mut next = changes.iter().peekable();
    let mut offset = 0;
    lines
        .iter()
        .map(|line| {
            while let Some(&&(ix, after)) = next.peek() {
                if ix >= offset {
                    break;
                }
                depth = after;
                next.next();
            }
            offset += line.len() + 1;
            depth
        })
        .collect()
}

/// Net brackets opened on one line, outside strings and trailing comments.
fn bracket_delta(line: &str) -> i64 {
    let mut delta = 0;
    let mut quote = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(_), '\\') => {
                chars.next();
            }
            (Some(open), _) if ch == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(ch),
            (None, '/') if chars.peek() == Some(&'/') => break,
            (None, '{' | '[' | '(') => delta += 1,
            (None, '}' | ']' | ')') => delta -= 1,
            _ => {}
        }
    }
    delta
}

/// Visit every code byte outside comments with the bracket depth before it.
/// A string is visited at its opening and closing quote only.
fn scan(src: &str, mut visit: impl FnMut(usize, u8, usize)) -> Result<(), String> {
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut ix = 0;
    while ix < bytes.len() {
        let byte = bytes[ix];
        match byte {
            b'"' | b'\'' => {
                visit(ix, byte, depth);
                ix += 1;
                while ix < bytes.len() && bytes[ix] != byte {
                    ix += if bytes[ix] == b'\\' { 2 } else { 1 };
                }
                if ix >= bytes.len() {
                    return Err("a string is missing its closing quote".to_string());
                }
                visit(ix, byte, depth);
            }
            b'/' if bytes.get(ix + 1) == Some(&b'/') => {
                while ix < bytes.len() && bytes[ix] != b'\n' {
                    ix += 1;
                }
                continue;
            }
            b'/' if bytes.get(ix + 1) == Some(&b'*') => {
                let Some(end) = src[ix + 2..].find("*/") else {
                    return Err("a /* comment is never closed".to_string());
                };
                ix += end + 4;
                continue;
            }
            b' ' | b'\t' | b'\n' | b'\r' => {}
            _ => {
                visit(ix, byte, depth);
                match byte {
                    b'[' | b'{' | b'(' => depth += 1,
                    b']' | b'}' | b')' => {
                        depth = depth.checked_sub(1).ok_or("there is an extra closing bracket")?;
                    }
                    _ => {}
                }
            }
        }
        ix += 1;
    }
    if depth == 0 { Ok(()) } else { Err("a bracket is never closed".to_string()) }
}

#[cfg(test)]
mod tests {
    use super::{parse_pipeline_text, pipeline_to_text};
    use crate::state::app_state::PipelineStage;

    fn stage(operator: &str, body: &str, enabled: bool) -> PipelineStage {
        PipelineStage::with(operator.to_string(), body.to_string(), enabled)
    }

    #[test]
    fn round_trips_disabled_and_unfinished_stages() {
        let stages = vec![
            stage("$match", "{\n  field: value\n}", true),
            stage("$group", "{\n  _id: \"$sensor\",\n  n: { $sum: 1 }\n}", false),
            stage("$project", "{ a: 1,\n  b: 2 }", true),
            stage("$limit", "10", true),
        ];
        let text = pipeline_to_text(&stages);
        assert!(text.contains("  // {\n  //   $group: {"));
        assert_eq!(parse_pipeline_text(&text).unwrap(), stages);
    }

    #[test]
    fn keeps_comments_inside_enabled_stages() {
        let text = "[\n  { $match: {\n    // only active\n    // { old: 1 }\n    status: 'active'\n  } },\n]";
        let stages = parse_pipeline_text(text).unwrap();
        assert_eq!(stages.len(), 1);
        assert!(stages[0].enabled);
        assert!(stages[0].body.contains("// only active"));
        assert!(stages[0].body.contains("// { old: 1 }"));
    }

    #[test]
    fn ignores_comments_between_stages() {
        let text = "[\n  // filter first\n  { $match: {} }, // why\n  // { $sort: { a: 1 } },\n  // note\n  { // inline\n    $limit: 5, // five\n  },\n]";
        let stages = parse_pipeline_text(text).unwrap();
        assert_eq!(
            stages,
            [
                stage("$match", "{}", true),
                stage("$sort", "{ a: 1 }", false),
                stage("$limit", "5", true)
            ]
        );
    }

    #[test]
    fn accepts_pasted_json() {
        let stages = parse_pipeline_text(r#"[{"$match":{"a":"x,y"}},{"$sort":{"a":-1}}]"#).unwrap();
        assert_eq!(stages[0], stage("$match", r#"{"a":"x,y"}"#, true));
        assert_eq!(stages[1], stage("$sort", r#"{"a":-1}"#, true));
        assert!(parse_pipeline_text("[]").unwrap().is_empty());
    }

    #[test]
    fn explains_what_is_wrong() {
        assert!(parse_pipeline_text("{ $match: {} }").unwrap_err().contains("array"));
        assert!(parse_pipeline_text("[ { $match: {} ]").is_err());
        assert!(
            parse_pipeline_text("[ { $match: {}, $sort: {} } ]").unwrap_err().contains("Stage 1")
        );
        assert!(parse_pipeline_text("[ 42 ]").unwrap_err().contains("Stage 1"));
        assert!(parse_pipeline_text("[ { $match: } ]").unwrap_err().contains("needs a value"));
    }
}
