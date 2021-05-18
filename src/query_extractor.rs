extern crate globwalk;
extern crate swc_common;
extern crate swc_ecma_parser;

use md5;
use std::path::Path;
use swc_atoms::JsWord;
use swc_common::sync::Lrc;
use swc_common::{
    errors::{ColorConfig, Handler},
    SourceMap,
};
use swc_ecma_ast::Decl::Var;
use swc_ecma_ast::Expr::TaggedTpl;
use swc_ecma_ast::ExprOrSuper::Expr;
use swc_ecma_ast::ImportSpecifier::Named;
use swc_ecma_ast::MemberExpr;
use swc_ecma_ast::Module;
use swc_ecma_ast::ModuleDecl::ExportDecl;
use swc_ecma_ast::ModuleDecl::Import;
use swc_ecma_ast::ModuleItem::ModuleDecl;
use swc_ecma_parser::JscTarget;

use swc_ecma_ast::Expr::{Ident, Member};
use swc_ecma_parser::{lexer::Lexer, EsConfig, Parser, StringInput, Syntax, TsConfig};
use swc_ecma_visit::{Node, Visit};

#[derive(Clone)]
pub struct SkippedResult {
    pub path: String,
    pub reason: String,
}

pub struct QueryExtractor<'a> {
    pub queries: &'a mut Vec<String>,
    pub skipped_files: &'a mut Vec<SkippedResult>,
    module: &'a Module,
    path: &'a Path,
}

fn is_graphql_tag(node: &swc_ecma_ast::TaggedTpl) -> bool {
    match &*node.tag {
        Member(m) => match &m.obj {
            Expr(e) => match &**e {
                Ident(t) => {
                    if t.sym == JsWord::from("Relay") {
                        match &*m.prop {
                            Ident(i) => i.sym == JsWord::from("QL"),
                            _ => false,
                        }
                    } else {
                        false
                    }
                }
                _ => false,
            },
            _ => false,
        },
        Ident(t) => t.sym == JsWord::from("gql"),
        _ => false,
    }
}

fn find_import_for_name(name: String, module: &Module) -> Option<String> {
    for item in module.body.iter() {
        match item {
            ModuleDecl(d) => match d {
                Import(i) => {
                    if i.specifiers.iter().any(|s| match s {
                        Named(n) => n.local.sym == name,
                        _ => false,
                    }) {
                        let mut path = i.src.value.to_string();
                        path.push_str(".js");
                        return Some(path);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    None
}

fn find_value_from_file(name: String, path: &Path) -> Option<String> {
    let module = get_ast_from_path(path).unwrap();

    for item in module.body.iter() {
        match item {
            ModuleDecl(d) => match d {
                ExportDecl(e) => match &e.decl {
                    Var(v) => {
                        // assuming we only declare one variable at the time
                        let decl = v.decls.get(0).unwrap();

                        if match &decl.name {
                            swc_ecma_ast::Pat::Ident(i) => i.sym == name,
                            _ => false,
                        } {
                            let value = decl.init.as_ref().unwrap();

                            match &**value {
                                TaggedTpl(tpl) => return Some(tpl.quasis[0].raw.value.to_string()),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            },
            _ => {}
        }
    }

    None
}

fn find_value_for_expr(
    name: String,
    module: &Module,
    module_path: &Path,
) -> Result<String, String> {
    match find_import_for_name(name.clone(), module) {
        Some(path) => {
            let imported_module_path = Path::new(&path);
            let abs_path = module_path
                .parent()
                .unwrap()
                .join(imported_module_path)
                .canonicalize();

            match abs_path {
                Ok(path) => {
                    let value = find_value_from_file(name.clone(), &path);

                    match value {
                        Some(t) => return Ok(t),
                        None => Err(format!("Unable to find value from import for {}", name)),
                    }
                }
                Err(e) => Err(format!("Got error when trying to find {}: {}", name, e)),
            }
        }
        None => Err(format!("Unable to find import for {}", name)),
    }
}

fn _find_name_for_expr(expr: &swc_ecma_ast::Expr) -> Vec<String> {
    match expr {
        Ident(i) => vec![i.sym.to_string()],
        Member(m) => _find_name_for_member(m),
        _ => vec![],
    }
}

fn _find_name_for_member(expr: &MemberExpr) -> Vec<String> {
    let mut parts = _find_name_for_expr(&*expr.prop);

    match &expr.obj {
        Expr(e) => {
            parts.append(&mut _find_name_for_expr(&e));
        }
        _ => {}
    };

    return parts;
}

fn find_value_for_member(
    _expr: &MemberExpr,
    _module: &Module,
    _module_path: &Path,
) -> Result<String, String> {
    // let mut parts = find_name_for_member(expr);
    // parts.reverse();

    // TODO: we don't support finding values for member expressions at
    // the moment so we just return an empty string for now 😊

    return Ok("".to_string());
}

impl Visit for QueryExtractor<'_> {
    fn visit_tagged_tpl(&mut self, n: &swc_ecma_ast::TaggedTpl, _parent: &dyn Node) {
        if is_graphql_tag(n) {
            let mut parts = Vec::<String>::new();

            // template literals are divided in quasis and expressions, see:
            // https://astexplorer.net/#/gist/56fa8c1b00bbf670fd06df091165cf07
            // we want to merge the quasis and replace the expressions with the
            // actual query content, usually fragments

            for (pos, quasi) in n.quasis.iter().enumerate() {
                parts.push(quasi.raw.value.to_string());

                if n.exprs.len() > pos {
                    let expr = &n.exprs[pos];

                    match &**expr {
                        Ident(i) => {
                            let name = i.sym.to_string();

                            let value = find_value_for_expr(name, self.module, self.path);

                            match value {
                                Ok(v) => parts.push(v),
                                Err(e) => {
                                    self.skipped_files.push(SkippedResult {
                                        path: self.path.display().to_string(),
                                        reason: e,
                                    });

                                    return;
                                }
                            }
                        }
                        Member(m) => match find_value_for_member(m, self.module, self.path) {
                            Ok(v) => parts.push(v),
                            Err(e) => {
                                self.skipped_files.push(SkippedResult {
                                    path: self.path.display().to_string(),
                                    reason: e,
                                });

                                return;
                            }
                        },
                        _ => {
                            self.skipped_files.push(SkippedResult {
                                path: self.path.display().to_string(),
                                reason: format!("Unsupported expression {:?}", expr),
                            });

                            return;
                        }
                    }
                }
            }

            let mut query = parts.join("").trim().to_string();

            // this adds names to anonymous fragments
            if query.starts_with("fragment on ") {
                let digest = md5::compute(query.clone());

                query = query.replace("fragment on ", &format!("fragment F_{:x} on ", digest));
            }

            self.queries.push(query);
        }
    }
}

fn get_ast_from_path(path: &Path) -> Result<Module, ()> {
    let cm: Lrc<SourceMap> = Default::default();
    let handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));
    let fm = cm.load_file(path).expect("Failed to load file.");

    let syntax = if path.extension().unwrap() == "js" {
        Syntax::Es(EsConfig {
            jsx: true,
            num_sep: true,
            class_private_props: true,
            class_private_methods: true,
            class_props: true,
            fn_bind: false,
            decorators: true,
            decorators_before_export: true,
            export_default_from: true,
            export_namespace_from: true,
            dynamic_import: true,
            nullish_coalescing: true,
            optional_chaining: true,
            import_meta: true,
            top_level_await: true,
            import_assertions: true,
        })
    } else {
        Syntax::Typescript(TsConfig {
            tsx: true,
            decorators: true,
            dynamic_import: true,
            dts: false,
            no_early_errors: true,
            import_assertions: false,
        })
    };

    let lexer = Lexer::new(syntax, JscTarget::Es2020, StringInput::from(&*fm), None);

    let mut parser = Parser::new_from(lexer);

    for e in parser.take_errors() {
        e.into_diagnostic(&handler).emit();
    }

    parser.parse_module().map_err(|e| {
        // Unrecoverable fatal error occurred
        e.into_diagnostic(&handler).emit()
    })
}

pub struct ExtractionResult {
    pub queries: Vec<String>,
    pub skipped_files: Vec<SkippedResult>,
}

pub fn extract_queries_from_file(path: &Path) -> Option<ExtractionResult> {
    let mut queries: Vec<String> = Vec::new();
    let mut skipped_files: Vec<SkippedResult> = Vec::new();
    let result = get_ast_from_path(path);

    match result {
        Ok(module) => {
            let mut extractor = QueryExtractor {
                path: path,
                queries: &mut queries,
                module: &module,
                skipped_files: &mut skipped_files,
            };

            extractor.visit_module(&module, &module);

            return Some(ExtractionResult {
                queries: queries.clone(),
                skipped_files: skipped_files.clone(),
            });
        }
        Err(_) => {
            println!("failed to parse module {}", path.display());
        }
    }

    None
}
