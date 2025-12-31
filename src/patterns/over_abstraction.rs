//! Over-Abstraction detection - layers of indirection with single implementations

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::db::{Database, Symbol, SymbolKind};

pub struct OverAbstractionDetector;

impl PatternDetector for OverAbstractionDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::OverAbstraction
    }

    fn detect(&self, db: &Database, _config: &Config) -> Vec<Finding> {
        let mut findings = Vec::new();

        // Get all symbols
        let all_files = match db.get_all_files() {
            Ok(files) => files,
            Err(_) => return findings,
        };

        // Look for patterns of over-abstraction:
        // 1. Interfaces with single implementations
        // 2. Wrapper functions that just call another function
        // 3. Factory functions that always return the same type

        // Check for wrapper functions
        for file in &all_files {
            let symbols = match db.get_file_symbols(file.id) {
                Ok(syms) => syms,
                Err(_) => continue,
            };

            for symbol in &symbols {
                if symbol.kind != SymbolKind::Function {
                    continue;
                }

                if let Some(finding) = detect_wrapper_function(symbol, &file.path, db) {
                    findings.push(finding);
                }
            }
        }

        // Check for single-implementation interfaces
        let interfaces = match db.get_symbols_by_kind(SymbolKind::Interface) {
            Ok(ints) => ints,
            Err(_) => return findings,
        };

        let classes = match db.get_symbols_by_kind(SymbolKind::Class) {
            Ok(cls) => cls,
            Err(_) => return findings,
        };

        for interface in &interfaces {
            if let Some(finding) = detect_single_impl_interface(interface, &classes, db) {
                findings.push(finding);
            }
        }

        findings
    }
}

fn detect_wrapper_function(func: &Symbol, file_path: &str, _db: &Database) -> Option<Finding> {
    let source = &func.source_text;
    let lines: Vec<_> = source.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with("/*"))
        .collect();

    // A wrapper function typically has:
    // - Very few lines (signature + return + closing brace)
    // - A single return statement that calls another function

    if lines.len() > 5 {
        return None;
    }

    // Check if body is just a function call
    let body_lines: Vec<_> = lines.iter()
        .filter(|l| !l.contains("function") && !l.starts_with("export") && **l != "{" && **l != "}" && **l != "};")
        .collect();

    if body_lines.len() != 1 {
        return None;
    }

    let body_line = body_lines[0];

    // Check for simple delegation patterns
    let is_delegation = body_line.starts_with("return ")
        && (body_line.contains("(") || body_line.ends_with(";"));

    // Check for arrow function wrappers
    let is_arrow_wrapper = body_line.contains("=>") && body_line.contains("(");

    if !is_delegation && !is_arrow_wrapper {
        return None;
    }

    // Extract the called function name if possible
    let called_func = extract_called_function(body_line);

    let title = format!("Wrapper function: {}", func.name);
    let description = format!(
        "`{}` appears to be a thin wrapper that delegates to `{}`. \
        Consider whether this indirection adds value.",
        func.name,
        called_func.as_deref().unwrap_or("another function")
    );

    let rationale = String::from(
        "Thin wrapper functions add indirection without value:\n\
        - Extra stack frames and function call overhead\n\
        - More code to read and understand\n\
        - Changes require updating multiple places\n\
        - Makes stack traces harder to read"
    );

    let location = Location::new(
        file_path.to_string(),
        func.start_line,
        func.end_line,
    )
    .with_snippet(func.source_text.clone())
    .with_context(format!("Wraps: {}", called_func.as_deref().unwrap_or("?")));

    let suggested_fix = format!(
        r#"1. **Evaluate if wrapper is needed**:
   - Does it add meaningful abstraction? (e.g., error handling, logging)
   - Is it for dependency injection/testing?
   - Does it adapt an interface?

2. **If not needed**:
   - Replace calls to `{}` with direct calls to `{}`
   - Remove the wrapper function

3. **If needed for testing/DI**:
   - Add a comment explaining why: `// Wrapper for testability`
   - Consider using dependency injection instead"#,
        func.name,
        called_func.as_deref().unwrap_or("the underlying function")
    );

    let mut finding = Finding::new(
        PatternType::OverAbstraction,
        Severity::Low,
        0.70,
        title,
        description,
    )
    .with_rationale(rationale)
    .with_locations(vec![location])
    .with_suggested_fix(suggested_fix);

    if let Some(called) = called_func {
        finding = finding.with_related_concepts(vec![func.name.clone(), called]);
    }

    Some(finding)
}

fn extract_called_function(line: &str) -> Option<String> {
    // Try to extract function name from patterns like:
    // return foo(args)
    // return this.foo(args)
    // () => foo(args)

    let cleaned = line
        .trim()
        .trim_start_matches("return ")
        .trim_start_matches("() => ")
        .trim_start_matches("async ")
        .trim_end_matches(';');

    // Find the function call
    if let Some(paren_pos) = cleaned.find('(') {
        let func_part = &cleaned[..paren_pos];
        // Get the last identifier (after any dots)
        let func_name = func_part.rsplit('.').next().unwrap_or(func_part);
        if !func_name.is_empty() && func_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Some(func_name.to_string());
        }
    }

    None
}

fn detect_single_impl_interface(
    interface: &Symbol,
    classes: &[Symbol],
    db: &Database,
) -> Option<Finding> {
    let interface_name = &interface.name;

    // Look for classes that might implement this interface
    // Common patterns: class FooImpl implements Foo, class FooService implements IFoo

    let potential_impls: Vec<_> = classes.iter()
        .filter(|cls| {
            let cls_source = cls.source_text.to_lowercase();
            let iface_lower = interface_name.to_lowercase();

            // Check if class implements this interface
            cls_source.contains(&format!("implements {}", iface_lower))
                || cls_source.contains(&format!("implements i{}", iface_lower))
                || cls_source.contains(&format!("implements {}interface", iface_lower))
        })
        .collect();

    // Only flag if exactly one implementation
    if potential_impls.len() != 1 {
        return None;
    }

    let impl_class = potential_impls[0];

    let interface_path = match db.get_file_path(interface.file_id) {
        Ok(Some(path)) => path,
        _ => return None,
    };

    let impl_path = match db.get_file_path(impl_class.file_id) {
        Ok(Some(path)) => path,
        _ => return None,
    };

    let title = format!("Single implementation: {}", interface_name);
    let description = format!(
        "Interface `{}` has only one implementation (`{}`). \
        This abstraction may be premature.",
        interface_name, impl_class.name
    );

    let rationale = String::from(
        "Interfaces with single implementations often indicate:\n\
        - Premature abstraction (YAGNI violation)\n\
        - Over-engineering for 'flexibility' that never materializes\n\
        - Extra cognitive overhead for readers\n\
        - More files to navigate and maintain"
    );

    let locations = vec![
        Location::new(interface_path.clone(), interface.start_line, interface.end_line)
            .with_context(format!("Interface: {}", interface_name)),
        Location::new(impl_path.clone(), impl_class.start_line, impl_class.end_line)
            .with_context(format!("Only implementation: {}", impl_class.name)),
    ];

    let suggested_fix = format!(
        r#"1. **Evaluate necessity**:
   - Will there ever be another implementation? (Be honest!)
   - Is this for dependency injection in tests?
   - Is this a public library API?

2. **If interface not needed**:
   - Merge `{}` into `{}`
   - Update all references to use the class directly
   - Delete the interface file

3. **If needed for testing**:
   - Consider using `jest.mock()` or similar instead
   - Or keep but document: `// Interface for DI/testing`

4. **If needed for future flexibility**:
   - Wait until you actually need the second implementation
   - It's easier to extract an interface later than to maintain one now"#,
        interface_name, impl_class.name
    );

    Some(Finding::new(
        PatternType::OverAbstraction,
        Severity::Low,
        0.65,
        title,
        description,
    )
    .with_rationale(rationale)
    .with_locations(locations)
    .with_suggested_fix(suggested_fix)
    .with_related_concepts(vec![interface_name.clone(), impl_class.name.clone()]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_called_function() {
        assert_eq!(extract_called_function("return foo(args)"), Some("foo".to_string()));
        assert_eq!(extract_called_function("return this.bar(x, y)"), Some("bar".to_string()));
        assert_eq!(extract_called_function("() => baz()"), Some("baz".to_string()));
    }
}
