use crate::{
  CompileOptions, HostOption,
  backend::codegen::{self, hosts::choose_host},
  backend::js::{self, print::PrintResult},
  check::{self, Types},
  core::{self, effects::EffectTable, ir::CModule, lower::Lowered},
  shared::{
    diagnostic::DiagnosticBag, modules::ModuleSource, source::SourceFile,
  },
  stdlib,
  syntax::ast::Module,
  syntax::builder::Expansion,
  syntax::expand,
  syntax::lexer,
  syntax::parser,
};

pub(crate) struct Analysed {
  pub module: Module,
  pub core: CModule,
  pub types: Types,
  pub effects: EffectTable,
}

pub(crate) fn front_end(
  file: &SourceFile,
  modules: &[ModuleSource],
  bag: &mut DiagnosticBag,
) -> Option<Analysed> {
  let mut module = parse(file, bag)?;
  let expansion = expand(&mut module, file, modules, bag);

  if bag.has_errors() {
    expansion.remap(bag);
    return None;
  }

  let lowered = lower(&module, modules, &expansion, bag);
  let types = check(&module, &lowered, bag);

  expansion.remap(bag);

  let types = types?;
  let core = elaborate(lowered.core, &types, modules, &expansion);

  Some(Analysed { module, core, types, effects: lowered.effects })
}

pub(crate) fn parse(
  file: &SourceFile,
  bag: &mut DiagnosticBag,
) -> Option<Module> {
  let lexed = lexer::lex(file, bag);
  let module = parser::parse(file, &lexed, bag);

  (!bag.has_errors()).then_some(module)
}

pub(crate) fn expand(
  module: &mut Module,
  file: &SourceFile,
  modules: &[ModuleSource],
  bag: &mut DiagnosticBag,
) -> Expansion {
  let root = module.name.as_ref().map(|n| n.text.clone());

  stdlib::within(root.as_deref(), || expand::expand(module, file, modules, bag))
}

pub(crate) fn lower(
  module: &Module,
  modules: &[ModuleSource],
  expansion: &Expansion,
  bag: &mut DiagnosticBag,
) -> Lowered {
  let root = module.name.as_ref().map(|n| n.text.as_str());

  stdlib::within(root, || {
    core::lower::lower_expanded(module, modules, expansion, bag)
  })
}

pub(crate) fn check(
  module: &Module,
  lowered: &Lowered,
  bag: &mut DiagnosticBag,
) -> Option<Types> {
  if bag.has_errors() {
    return None;
  }

  let types = check::check(module, lowered, bag);

  (!bag.has_errors()).then_some(types)
}

pub(crate) fn elaborate(
  core: CModule,
  types: &Types,
  modules: &[ModuleSource],
  expansion: &Expansion,
) -> CModule {
  let core = core::annotate::annotate(core, types);
  let mut core = core::dictionaries::elaborate(core, types, modules);

  expansion.remap_core(&mut core);
  core
}

pub(crate) fn host(
  options: &CompileOptions,
  effects: &EffectTable,
) -> Result<Option<String>, String> {
  let declared = effects.host_names();

  match &options.host {
    HostOption::Auto => choose_host(None, &declared),
    HostOption::Flag(flag) => choose_host(Some(flag), &declared),
    HostOption::Fixed(host) => Ok(host.clone()),
  }
}

pub(crate) fn emit(
  core: CModule,
  file: &SourceFile,
  options: &CompileOptions,
) -> PrintResult {
  let core = core::matching::compile_matches(core);
  let program = codegen::emit::emit(&core, file, options);

  js::print::print_program(&program)
}
