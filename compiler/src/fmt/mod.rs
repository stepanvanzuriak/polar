pub mod comments;
pub mod doc;
pub mod parens;
pub mod print;

use crate::{
  shared::diagnostic::{Diagnostic, DiagnosticBag},
  shared::ice::ice,
  shared::source::SourceFile,
  syntax::ast::eq::ast_diff,
  syntax::lexer::lex,
  syntax::parser::{parse, precedence::PrecedenceTable},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatResult {
  pub output: Option<String>,
  pub diagnostics: Vec<Diagnostic>,
}

impl FormatResult {
  #[must_use]
  pub fn is_ok(&self) -> bool {
    self.output.is_some()
  }
}

#[must_use]
pub fn format(source: &str, filename: &str) -> FormatResult {
  format_with(source, filename, |_| {})
}

#[must_use]
pub fn format_plugins(
  source: &str,
  filename: &str,
  plugins: &[String],
) -> FormatResult {
  crate::syntax::plugins::within(plugins, || format(source, filename))
}

#[doc(hidden)]
#[must_use]
pub fn format_with(
  source: &str,
  filename: &str,
  patch: impl FnOnce(&mut String),
) -> FormatResult {
  let file = SourceFile::new(filename, source);
  let mut bag = DiagnosticBag::default();
  let lexed = lex(&file, &mut bag);
  let module = parse(&file, &lexed, &mut bag);

  if bag.has_errors() {
    return FormatResult { output: None, diagnostics: bag.into_sorted() };
  }

  let table = PrecedenceTable::default();
  let mut output =
    print::print_module(&module, source, &lexed.comments, &table);

  patch(&mut output);

  let printed = SourceFile::new(filename, output.as_str());
  let mut rebag = DiagnosticBag::default();
  let relexed = lex(&printed, &mut rebag);

  if relexed.comments.len() != lexed.comments.len() {
    ice(
      format!(
        "formatting {filename} changed its comment count from {} to {}",
        lexed.comments.len(),
        relexed.comments.len()
      ),
      None,
    );
  }

  let reparsed = parse(&printed, &relexed, &mut rebag);

  if rebag.has_errors() {
    ice(
      format!("formatting {filename} produced output that does not parse"),
      None,
    );
  }

  if let Some(diff) = ast_diff(&module, &reparsed) {
    ice(format!("formatting {filename} changed its meaning: {diff}"), None);
  }

  FormatResult { output: Some(output), diagnostics: bag.into_sorted() }
}
