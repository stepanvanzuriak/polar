use crate::files::{Input, normalize};
use std::{
  fmt::Write as _,
  fs,
  path::{Path, PathBuf},
};

const MARK: &str = "// expect-error:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expectation {
  pub code: String,
  pub message: Option<String>,
  pub line: usize,
  pub written_at: usize,
}

pub(crate) fn is_expected_failure(text: &str) -> bool {
  text.lines().any(|line| line.trim_start().starts_with(MARK))
}

pub(crate) fn without_expected_errors(
  inputs: Vec<Input>,
  args: &[PathBuf],
  cwd: &Path,
) -> Vec<Input> {
  let named: Vec<PathBuf> =
    args.iter().map(|arg| normalize(&cwd.join(arg))).collect();

  inputs
    .into_iter()
    .filter(|input| {
      named.contains(&normalize(&cwd.join(&input.path)))
        || !fs::read_to_string(&input.path)
          .is_ok_and(|text| is_expected_failure(&text))
    })
    .collect()
}

/// Reads the `// expect-error: CODE "text"` comments of a file. Each applies to
/// the next line that is not blank and not another expectation.
///
/// # Errors
///
/// Fails with a message and the 1-based line of a malformed expectation.
pub fn parse(text: &str) -> Result<Vec<Expectation>, (usize, String)> {
  let lines: Vec<&str> = text.lines().collect();
  let mut out = Vec::new();

  for (i, line) in lines.iter().enumerate() {
    let Some(rest) = line.trim_start().strip_prefix(MARK) else { continue };
    let rest = rest.trim();
    let (code, tail) =
      rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let code = code.to_ascii_uppercase();

    if !code.starts_with("POLAR")
      || code.len() <= 5
      || !code[5..].chars().all(|c| c.is_ascii_digit())
    {
      return Err((
        i + 1,
        format!(
          "expected `// expect-error: POLAR0000 \"text\"`, found `{}`",
          line.trim()
        ),
      ));
    }

    let tail = tail.trim();
    let message = if tail.is_empty() {
      None
    } else {
      match (tail.find('"'), tail.rfind('"')) {
        (Some(start), Some(end)) if end > start => {
          Some(tail[start + 1..end].to_string())
        }
        _ => {
          return Err((
            i + 1,
            "the message after the code must be in double quotes".to_string(),
          ));
        }
      }
    };
    let target = lines.iter().enumerate().skip(i + 1).find(|(_, l)| {
      let l = l.trim();

      !l.is_empty() && !l.starts_with(MARK)
    });

    let Some((target, _)) = target else {
      return Err((
        i + 1,
        "this expectation is not followed by a line of code".to_string(),
      ));
    };

    out.push(Expectation {
      code,
      message,
      line: target + 1,
      written_at: i + 1,
    });
  }

  Ok(out)
}

/// What differs between the expectations and the errors found, one sentence
/// per problem; empty when the file fails exactly as expected.
#[must_use]
pub fn mismatches(
  expected: &[Expectation],
  found: &[(String, usize, String)],
) -> Vec<String> {
  let mut unused: Vec<&(String, usize, String)> = found.iter().collect();
  let mut problems = Vec::new();

  for want in expected {
    let hit = unused.iter().position(|(code, line, message)| {
      *code == want.code
        && *line == want.line
        && want.message.as_ref().is_none_or(|text| message.contains(text))
    });

    if let Some(i) = hit {
      unused.remove(i);
    } else {
      {
        let same_code = unused.iter().find(|(code, ..)| *code == want.code);
        let text = want
          .message
          .as_ref()
          .map_or(String::new(), |m| format!(" containing \"{m}\""));

        problems.push(match same_code {
          Some((_, line, message)) => format!(
            "expected {} at line {}{text}, but {} is at line {line}: {}",
            want.code,
            want.line,
            want.code,
            message.lines().next().unwrap_or_default()
          ),
          None => format!(
            "expected {} at line {}{text}, but it was not reported",
            want.code, want.line
          ),
        });
      }
    }
  }

  for (code, line, message) in unused {
    problems.push(format!(
      "unexpected {code} at line {line}: {}",
      message.lines().next().unwrap_or_default()
    ));
  }

  problems
}

pub(crate) fn bless(
  path: &Path,
  text: &str,
  found: &[(String, usize, String)],
) -> std::io::Result<()> {
  let mut out = String::new();

  for (i, line) in text.split_inclusive('\n').enumerate() {
    if line.trim_start().starts_with(MARK) {
      continue;
    }

    let indent: String =
      line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();

    for (code, at, message) in found {
      if *at == i + 1 {
        let first = message.lines().next().unwrap_or_default();

        let _ = writeln!(out, "{indent}{MARK} {code} \"{first}\"");
      }
    }

    out.push_str(line);
  }

  crate::files::write_atomic(path, out.as_bytes())
}
