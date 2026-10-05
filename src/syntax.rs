use crate::workspace::SymbolCandidate;
use std::path::Path;
pub fn symbols(text: &str, relative: &str) -> Option<Vec<SymbolCandidate>> {
    let language = match Path::new(relative).extension()?.to_str()? {
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        "py" => tree_sitter_python::LANGUAGE.into(),
        "ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        _ => return None,
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(text, None)?;
    let lines = text.lines().collect::<Vec<_>>();
    let mut pending = vec![tree.root_node()];
    let mut output = Vec::new();
    while let Some(node) = pending.pop() {
        let symbol = matches!(
            node.kind(),
            "function_item"
                | "struct_item"
                | "enum_item"
                | "trait_item"
                | "impl_item"
                | "function_definition"
                | "class_definition"
                | "function_declaration"
                | "method_definition"
                | "class_declaration"
                | "interface_declaration"
                | "type_alias_declaration"
                | "method_declaration"
                | "type_declaration"
        ) || node.kind() == "variable_declarator"
            && node.child_by_field_name("value").is_some_and(|value| {
                matches!(value.kind(), "arrow_function" | "function_expression")
            });
        if symbol {
            let start_line = node.start_position().row + 1;
            let end = node.end_position();
            let end_line = if end.column == 0 {
                end.row.max(start_line)
            } else {
                end.row + 1
            };
            output.push(SymbolCandidate {
                path: relative.to_owned(),
                label: lines
                    .get(start_line - 1)
                    .copied()
                    .unwrap_or_default()
                    .trim()
                    .chars()
                    .take(240)
                    .collect(),
                start_line,
                end_line,
            });
        }
        let mut cursor = node.walk();
        pending.extend(
            node.named_children(&mut cursor)
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
    }
    Some(output)
}
/// One-hop lexical import hints; never loads packages or executes source.
pub fn imports(text: &str) -> Vec<String> {
    static IMPORT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:use|mod|from|import)\s+([\w./:]+)|(?:from\s+|import\s*\(?)['"]([.\w/-]+)['"]"#).expect("import regex")
    });
    IMPORT
        .captures_iter(text)
        .filter_map(|capture| capture.get(1).or_else(|| capture.get(2)))
        .map(|part| {
            part.as_str()
                .trim_start_matches("crate::")
                .trim_start_matches("self::")
                .replace("::", "/")
                .replace('.', "/")
                .trim_matches('/')
                .to_owned()
        })
        .filter(|hint| {
            !hint.is_empty()
                && !["std", "core", "alloc"]
                    .iter()
                    .any(|prefix| hint == *prefix || hint.starts_with(&format!("{prefix}/")))
        })
        .take(128)
        .collect()
}
pub fn imports_path(imports: &[String], path: &str) -> bool {
    let path = path.replace('\\', "/");
    let stem = path.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(&path);
    imports.iter().any(|import| {
        let module = import.split('/').next().unwrap_or(import);
        stem.ends_with(&format!("/{import}"))
            || stem == *import
            || stem.ends_with(&format!("/{module}"))
            || stem.ends_with(&format!("/{module}/mod"))
            || path.contains(&format!("/{import}/"))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiline_and_late_rust_symbols() {
        let source = format!(
            "pub(crate) async fn login(\n input: String,\n) {{\n println!(\"{{}}\", input);\n}}\n{}",
            (0..100)
                .map(|n| format!("fn worker_{n}() {{}}\n"))
                .collect::<String>()
        );
        let found = symbols(&source, "src/lib.rs").unwrap();
        assert_eq!(found.len(), 101);
        assert_eq!(found[0].end_line, 5);
        assert!(found.last().unwrap().label.contains("worker_99"));
    }
    #[test]
    fn languages_and_imports() {
        for (file, source) in [
            ("test.py", "async def login():\n    pass\n"),
            ("test.go", "package test\nfunc Login() {}\n"),
            ("test.ts", "export const login = async () => true;\n"),
        ] {
            assert!(!symbols(source, file).unwrap().is_empty());
        }
        assert!(imports_path(
            &imports("use crate::auth::login;\n"),
            "src/auth.rs"
        ));
    }
}
