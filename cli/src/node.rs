use std::path::PathBuf;

pub(crate) fn locate(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
  if let Some(node) = env("POLAR_NODE") {
    let node = PathBuf::from(node);

    return node.is_file().then_some(node);
  }

  let path = env("PATH")?;

  std::env::split_paths(&path)
    .flat_map(|dir| [dir.join("node"), dir.join("node.exe")])
    .find(|candidate| candidate.is_file())
}
