# Polar for VS Code

Syntax highlighting for Polar (`.px`) files.

Covers zone keywords (`module`, `uses`, `types`, `constants`, `functions`,
`exports`), keywords, `//` and `///` doc comments, strings with `#{…}` interpolation
and escapes, numbers with `_` separators and exponents, operators (`|>`, `->`, `..`),
and type / constructor names.

## Install locally

Symlink the folder into your VS Code extensions directory and reload the window:

```sh
ln -s "$PWD/editor/vscode" ~/.vscode/extensions/polar-lang
```

Or package it:

```sh
cd editor/vscode
npx @vscode/vsce package
code --install-extension polar-lang-0.0.1.vsix
```
