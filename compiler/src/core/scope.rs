use crate::core::ir::Sym;

#[derive(Debug, Default)]
pub struct Scopes {
  frames: Vec<Vec<(String, Sym)>>,
}

impl Scopes {
  pub fn push(&mut self) {
    self.frames.push(Vec::new());
  }

  pub fn pop(&mut self) {
    self.frames.pop();
  }

  pub fn bind(&mut self, name: &str, sym: Sym) {
    match self.frames.last_mut() {
      Some(frame) => frame.push((name.to_string(), sym)),
      None => {
        crate::shared::ice::ice("bound a name with no scope pushed", None)
      }
    }
  }

  #[must_use]
  pub fn lookup(&self, name: &str) -> Option<&Sym> {
    self
      .frames
      .iter()
      .rev()
      .flat_map(|frame| frame.iter().rev())
      .find(|(bound, _)| bound == name)
      .map(|(_, sym)| sym)
  }

  pub fn names(&self) -> impl Iterator<Item = &str> {
    self.frames.iter().flatten().map(|(name, _)| name.as_str())
  }
}

pub fn suggest<'a>(
  name: &str,
  candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
  candidates
    .into_iter()
    .filter(|&candidate| candidate != name)
    .map(|candidate| (levenshtein(name, candidate), candidate))
    .filter(|&(distance, _)| distance <= 2)
    .min()
    .map(|(_, candidate)| candidate)
}

#[must_use]
pub fn levenshtein(a: &str, b: &str) -> usize {
  let b: Vec<char> = b.chars().collect();
  let mut prev: Vec<usize> = (0..=b.len()).collect();
  let mut row = vec![0; b.len() + 1];

  for (i, ca) in a.chars().enumerate() {
    row[0] = i + 1;

    for (j, &cb) in b.iter().enumerate() {
      let substitute = prev[j] + usize::from(ca != cb);

      row[j + 1] = substitute.min(prev[j + 1] + 1).min(row[j] + 1);
    }

    std::mem::swap(&mut prev, &mut row);
  }

  prev[b.len()]
}
