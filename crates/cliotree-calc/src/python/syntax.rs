//! Checking Python formula text without running it (DESIGN.md, "Syntax
//! errors").
//!
//! This uses the same parser that RustPython uses, from the `ruff` fork, but
//! not the interpreter, so it works on any thread, and in a workbook that is
//! not trusted. Besides parsing, it rejects double-underscore access
//! (DESIGN.md, "Python engine", "What the sandbox really is"). That check
//! catches mistakes; it is not a security boundary.

use ruff_python_ast::visitor::{Visitor, walk_expr};
use ruff_python_ast::{Expr, ExprAttribute, ExprName, Stmt};
use ruff_python_parser::{parse_expression, parse_module};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextSize};

use crate::engine::{ModuleInfo, Syntax, SyntaxError};

/// The syntax check for Python formulas.
#[derive(Debug, Default, Clone, Copy)]
pub struct PythonSyntax;

impl Syntax for PythonSyntax {
    fn check(&self, formula: &str) -> Result<(), SyntaxError> {
        let text = formula.trim();
        if text.is_empty() {
            return Err(SyntaxError {
                message: "the formula is empty".to_owned(),
                line: 1,
                column: 1,
            });
        }
        let parsed = parse_expression(text)
            .map_err(|e| error_at(text, e.location.start(), e.error.to_string()))?;
        let mut scan = DunderScan { found: None };
        scan.visit_expr(&parsed.syntax().body);
        match scan.found {
            None => Ok(()),
            Some((offset, name)) => Err(error_at(
                text,
                offset,
                format!("double-underscore access is not allowed: `{name}`"),
            )),
        }
    }

    fn module_info(&self, source: &str) -> Result<ModuleInfo, SyntaxError> {
        PythonSyntax::read_module(source)
    }
}

impl PythonSyntax {
    /// Reads a module: it must parse and must not use double-underscore
    /// access, and what it imports and defines at the top level is listed.
    fn read_module(source: &str) -> Result<ModuleInfo, SyntaxError> {
        let parsed = parse_module(source)
            .map_err(|e| error_at(source, e.location.start(), e.error.to_string()))?;
        let body = &parsed.syntax().body;
        let mut scan = DunderScan { found: None };
        for stmt in body {
            scan.visit_stmt(stmt);
        }
        if let Some((offset, name)) = scan.found {
            return Err(error_at(
                source,
                offset,
                format!("double-underscore access is not allowed: `{name}`"),
            ));
        }
        let first = |name: &str| name.split('.').next().unwrap_or_default().to_owned();
        let mut info = ModuleInfo::default();
        for stmt in body {
            match stmt {
                Stmt::Import(import) => {
                    for alias in &import.names {
                        info.imports.push(first(alias.name.as_str()));
                    }
                }
                Stmt::ImportFrom(import) if import.level == 0 => {
                    if let Some(module) = &import.module {
                        info.imports.push(first(module.as_str()));
                    }
                }
                Stmt::FunctionDef(function) => {
                    info.functions.push(function.name.as_str().to_owned());
                }
                _ => {}
            }
        }
        Ok(info)
    }
}

fn error_at(text: &str, offset: TextSize, message: String) -> SyntaxError {
    let index = LineIndex::from_source_text(text);
    let at = index.line_column(offset, text);
    SyntaxError {
        message,
        line: at.line.get(),
        column: at.column.get(),
    }
}

struct DunderScan {
    found: Option<(TextSize, String)>,
}

impl<'a> Visitor<'a> for DunderScan {
    fn visit_expr(&mut self, expr: &'a Expr) {
        if self.found.is_some() {
            return;
        }
        match expr {
            Expr::Attribute(ExprAttribute { attr, .. }) if attr.as_str().starts_with("__") => {
                self.found = Some((attr.start(), attr.as_str().to_owned()));
                return;
            }
            Expr::Name(ExprName { id, .. }) if id.as_str().starts_with("__") => {
                self.found = Some((expr.start(), id.as_str().to_owned()));
                return;
            }
            _ => {}
        }
        walk_expr(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(text: &str) -> Result<(), SyntaxError> {
        PythonSyntax.check(text)
    }

    #[test]
    fn accepts_expressions_with_lambdas_comprehensions_and_line_breaks_inside_brackets() {
        for ok in [
            "1 + 2",
            "  SUM(Orders.items.price)  ",
            "[x * 2 for x in Orders.items if x.qty > 1]",
            "IFERROR(lambda: Orders.total / Orders.count, 0)",
            "f\"{Orders.name!r:>10}\"",
            "(1 +\n 2)",
            "x if (x := 3) > 2 else 0",
            "Orders.items[-1].price",
            "Orders.items[1:3]",
            "{'a': 1, 'b': [1, 2]}",
        ] {
            assert_eq!(check(ok), Ok(()), "{ok}");
        }
    }

    /// Formulas, "Syntax errors": the message says where.
    #[test]
    fn a_syntax_error_says_what_and_where() {
        let e = check("1 +").unwrap_err();
        assert_eq!((e.line, e.column), (1, 4), "{e}");
        let e = check("(1,\n 2))").unwrap_err();
        assert_eq!(e.line, 2, "{e}");
        let e = check("").unwrap_err();
        assert!(e.message.contains("empty"));
        assert!(check("   ").is_err());
    }

    #[test]
    fn statements_are_not_expressions() {
        assert!(check("x = 1").is_err());
        assert!(check("import os").is_err());
        assert!(check("1; 2").is_err());
        assert!(check("def f(): pass").is_err());
    }

    #[test]
    fn a_module_is_read_for_its_imports_and_functions_and_must_parse() {
        let info = PythonSyntax
            .module_info(
                "import math\nimport Tax.rates as r\nfrom Base import helper\n\n\
                 def vat(x):\n    return x * 0.21\n\ndef _private():\n    pass\n\
                 class C:\n    def method(self):\n        pass\n",
            )
            .unwrap();
        assert_eq!(info.imports, ["math", "Tax", "Base"]);
        assert_eq!(info.functions, ["vat", "_private"]);

        let e = PythonSyntax.module_info("def f(:\n  pass\n").unwrap_err();
        assert_eq!(e.line, 1, "{e}");
        let e = PythonSyntax
            .module_info("def f(x):\n    return x.__class__\n")
            .unwrap_err();
        assert!(e.message.contains("double-underscore"), "{e}");
        assert_eq!(PythonSyntax.module_info(""), Ok(ModuleInfo::default()));
    }

    /// Python engine: double-underscore access is rejected when a formula is
    /// entered, and the message says so.
    #[test]
    fn double_underscore_access_is_rejected() {
        for bad in [
            "().__class__",
            "x.__class__.__base__.__subclasses__()",
            "__import__('os')",
            "[a.__dict__ for a in b]",
            "(lambda: ().__class__)()",
            "f'{x.__class__}'",
            "__builtins__",
        ] {
            let e = check(bad).unwrap_err();
            assert!(e.message.contains("double-underscore"), "{bad}: {e}");
        }
        // A name that merely contains underscores is fine.
        assert_eq!(check("my__name + _private + trailing_"), Ok(()));
    }
}
