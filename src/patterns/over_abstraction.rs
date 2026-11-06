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

        // Look for patterns of over-abstraction:
        // 1. Interfaces with single implementations
        // 2. Wrapper functions that only forward their parameters (the structural rule
        //    in structural.rs checks arguments against parameters)
        // 3. Factory functions that always return the same type

        if let Ok(notes) = db.structural_notes("wrapper") {
            for note in notes {
                if is_low_signal_path(&note.file_path) {
                    continue;
                }
                let title = format!("Wrapper function: {}", note.name);
                let description = format!(
                    "`{}` matches a pass-through wrapper around `{}`.",
                    note.name,
                    if note.detail.is_empty() {
                        "another function"
                    } else {
                        &note.detail
                    }
                );
                let finding = Finding::new(
                    PatternType::OverAbstraction,
                    Severity::Low,
                    0.82,
                    title,
                    description,
                )
                .with_rationale(
                    "ast-grep matched a function whose body only forwards to another call."
                        .to_string(),
                )
                .with_locations(vec![Location::new(note.file_path, note.line, note.line)
                    .with_context(format!("Wraps {}", note.detail))])
                .with_suggested_fix(
                    "Inline the wrapper or keep it only when it adds a real boundary.".to_string(),
                )
                .with_related_concepts(vec![note.name, note.detail]);
                findings.push(finding);
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

        findings.truncate(40);
        findings
    }
}

fn is_low_signal_path(path: &str) -> bool {
    path.contains("/components/ui/")
        || path.contains("/ui/")
        || path.contains("/testing/")
        || path.contains("/seed/")
        || path.contains("/_generated/")
        || path.contains("/patches/")
}

fn detect_single_impl_interface(
    interface: &Symbol,
    classes: &[Symbol],
    db: &Database,
) -> Option<Finding> {
    let interface_name = &interface.name;

    // Look for classes that might implement this interface
    // Common patterns: class FooImpl implements Foo, class FooService implements IFoo

    let potential_impls: Vec<_> = classes
        .iter()
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
        - More files to navigate and maintain",
    );

    let locations = vec![
        Location::new(
            interface_path.clone(),
            interface.start_line,
            interface.end_line,
        )
        .with_context(format!("Interface: {}", interface_name)),
        Location::new(
            impl_path.clone(),
            impl_class.start_line,
            impl_class.end_line,
        )
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

    Some(
        Finding::new(
            PatternType::OverAbstraction,
            Severity::Low,
            0.65,
            title,
            description,
        )
        .with_rationale(rationale)
        .with_locations(locations)
        .with_suggested_fix(suggested_fix)
        .with_related_concepts(vec![interface_name.clone(), impl_class.name.clone()]),
    )
}
