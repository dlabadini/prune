//! Structural parsing of TypeScript interfaces and object type aliases.

use crate::db::SymbolKind;
use oxc_allocator::Allocator;
use oxc_ast::ast::{Statement, TSSignature, TSType, TSTypeName};
use oxc_parser::Parser as OxcParser;
use oxc_span::SourceType;

#[derive(Debug, Clone)]
pub struct ParsedTypeDef {
    pub name: String,
    pub kind: SymbolKind,
    pub fields: Vec<TypeField>,
    pub extends: Vec<String>,
    pub is_object_like: bool,
    pub is_boilerplate: bool,
}

#[derive(Debug, Clone)]
pub struct TypeField {
    pub name: String,
    pub optional: bool,
    pub shape: TypeShape,
}

#[derive(Debug, Clone)]
pub enum TypeShape {
    AnonymousObject(Vec<TypeField>),
    Primitive(String),
    Reference(String),
    Union(Vec<TypeShape>),
    Intersection(Vec<TypeShape>),
    Array(Box<TypeShape>),
    Tuple(Vec<TypeShape>),
    Function {
        params: Vec<String>,
        returns: String,
    },
    Literal(String),
    Other(String),
}

#[derive(Debug, Clone)]
pub struct FieldBinding {
    pub name: String,
    pub signature: String,
    pub optional: bool,
    pub nested: Option<NestedSlot>,
}

#[derive(Debug, Clone)]
pub struct NestedSlot {
    pub field_name: String,
    pub signature: String,
    pub fields: Vec<FieldBinding>,
}

pub fn parse_type_definition(source: &str, name: &str, kind: SymbolKind) -> Option<ParsedTypeDef> {
    let allocator = Allocator::default();
    let parser = OxcParser::new(&allocator, source, SourceType::tsx());
    let result = parser.parse();
    if !result.diagnostics.is_empty() {
        return None;
    }

    for statement in &result.program.body {
        if let Some(parsed) = parse_statement(statement, name, kind) {
            return Some(parsed);
        }
    }
    None
}

fn parse_statement(statement: &Statement<'_>, name: &str, kind: SymbolKind) -> Option<ParsedTypeDef> {
    match statement {
        Statement::TSInterfaceDeclaration(interface) if interface.id.name.as_str() == name => {
            let extends = interface
                .extends
                .iter()
                .filter_map(|ext| type_name_from_type_name(&ext.type_name))
                .collect();
            let fields: Vec<TypeField> = interface
                .body
                .body
                .iter()
                .filter_map(parse_signature)
                .collect();
            let is_object_like = !fields.is_empty();
            Some(ParsedTypeDef {
                name: name.to_string(),
                kind,
                fields,
                extends,
                is_object_like,
                is_boilerplate: false,
            })
        }
        Statement::TSTypeAliasDeclaration(alias) if alias.id.name.as_str() == name => {
            let shape = parse_type(&alias.type_annotation);
            let (fields, is_object_like) = match shape {
                TypeShape::AnonymousObject(fields) => (fields, true),
                TypeShape::Union(shapes) => {
                    if is_ctx_union(&shapes) {
                        return Some(ParsedTypeDef {
                            name: name.to_string(),
                            kind,
                            fields: Vec::new(),
                            extends: Vec::new(),
                            is_object_like: false,
                            is_boilerplate: true,
                        });
                    }
                    (Vec::new(), false)
                }
                TypeShape::Reference(reference) if reference.starts_with("Infer<") => {
                    return Some(ParsedTypeDef {
                        name: name.to_string(),
                        kind,
                        fields: Vec::new(),
                        extends: Vec::new(),
                        is_object_like: false,
                        is_boilerplate: true,
                    });
                }
                _ => (Vec::new(), false),
            };
            Some(ParsedTypeDef {
                name: name.to_string(),
                kind,
                fields,
                extends: Vec::new(),
                is_object_like,
                is_boilerplate: false,
            })
        }
        _ => None,
    }
}

fn parse_signature(signature: &TSSignature<'_>) -> Option<TypeField> {
    match signature {
        TSSignature::TSPropertySignature(prop) => {
            let field_name = match &prop.key {
                oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str().to_string(),
                oxc_ast::ast::PropertyKey::Identifier(id) => id.name.as_str().to_string(),
                oxc_ast::ast::PropertyKey::StringLiteral(lit) => lit.value.to_string(),
                _ => return None,
            };
            let optional = prop.optional;
            let shape = prop
                .type_annotation
                .as_ref()
                .map(|annotation| parse_type(&annotation.type_annotation))
                .unwrap_or(TypeShape::Other("unknown".to_string()));
            Some(TypeField {
                name: field_name,
                optional,
                shape,
            })
        }
        _ => None,
    }
}

fn parse_type(ty: &TSType<'_>) -> TypeShape {
    match ty {
        TSType::TSTypeLiteral(object) => TypeShape::AnonymousObject(
            object
                .members
                .iter()
                .filter_map(parse_signature)
                .collect(),
        ),
        TSType::TSTypeReference(reference) => {
            TypeShape::Reference(reference_type_name(reference))
        }
        TSType::TSUnionType(union_type) => TypeShape::Union(
            union_type
                .types
                .iter()
                .map(parse_type)
                .collect(),
        ),
        TSType::TSIntersectionType(intersection) => TypeShape::Intersection(
            intersection
                .types
                .iter()
                .map(parse_type)
                .collect(),
        ),
        TSType::TSArrayType(array) => TypeShape::Array(Box::new(parse_type(&array.element_type))),
        TSType::TSTupleType(tuple) => TypeShape::Tuple(
            tuple
                .element_types
                .iter()
                .map(parse_tuple_element)
                .collect(),
        ),
        TSType::TSFunctionType(function) => {
            let params = function
                .params
                .items
                .iter()
                .map(|param| {
                    param
                        .type_annotation
                        .as_ref()
                        .map(|annotation| shape_signature(&parse_type(&annotation.type_annotation)))
                        .unwrap_or_else(|| "unknown".to_string())
                })
                .collect();
            let returns =
                shape_signature(&parse_type(&function.return_type.type_annotation));
            TypeShape::Function { params, returns }
        }
        TSType::TSStringKeyword(_) => TypeShape::Primitive("string".to_string()),
        TSType::TSNumberKeyword(_) => TypeShape::Primitive("number".to_string()),
        TSType::TSBooleanKeyword(_) => TypeShape::Primitive("boolean".to_string()),
        TSType::TSBigIntKeyword(_) => TypeShape::Primitive("bigint".to_string()),
        TSType::TSUnknownKeyword(_) => TypeShape::Primitive("unknown".to_string()),
        TSType::TSAnyKeyword(_) => TypeShape::Primitive("any".to_string()),
        TSType::TSVoidKeyword(_) => TypeShape::Primitive("void".to_string()),
        TSType::TSNullKeyword(_) => TypeShape::Primitive("null".to_string()),
        TSType::TSUndefinedKeyword(_) => TypeShape::Primitive("undefined".to_string()),
        TSType::TSNeverKeyword(_) => TypeShape::Primitive("never".to_string()),
        TSType::TSLiteralType(literal) => {
            TypeShape::Literal(ts_literal_signature(&literal.literal))
        }
        TSType::TSTypeQuery(_) => TypeShape::Other("type_query".to_string()),
        TSType::TSParenthesizedType(parenthesized) => parse_type(&parenthesized.type_annotation),
        _ => TypeShape::Other("complex".to_string()),
    }
}

fn parse_tuple_element(element: &oxc_ast::ast::TSTupleElement<'_>) -> TypeShape {
    use oxc_ast::ast::TSTupleElement;
    match element {
        TSTupleElement::TSTypeLiteral(literal) => TypeShape::AnonymousObject(
            literal
                .members
                .iter()
                .filter_map(parse_signature)
                .collect(),
        ),
        TSTupleElement::TSTypeReference(reference) => {
            TypeShape::Reference(reference_type_name(reference))
        }
        TSTupleElement::TSUnionType(union_type) => TypeShape::Union(
            union_type
                .types
                .iter()
                .map(parse_type)
                .collect(),
        ),
        TSTupleElement::TSArrayType(array) => {
            TypeShape::Array(Box::new(parse_type(&array.element_type)))
        }
        TSTupleElement::TSOptionalType(optional) => parse_type(&optional.type_annotation),
        TSTupleElement::TSRestType(rest) => {
            TypeShape::Array(Box::new(parse_type(&rest.type_annotation)))
        }
        _ => TypeShape::Other("tuple".to_string()),
    }
}

fn reference_type_name(reference: &oxc_ast::ast::TSTypeReference<'_>) -> String {
    let base = type_name_from_type_name(&reference.type_name)
        .unwrap_or_else(|| "Unknown".to_string());
    if reference.type_arguments.is_none() {
        return base;
    }
    let args = reference
        .type_arguments
        .as_ref()
        .map(|arguments| {
            arguments
                .params
                .iter()
                .map(|arg| shape_signature(&parse_type(arg)))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    format!("{}<{}>", base, args)
}

fn type_name_from_type_name(name: &TSTypeName<'_>) -> Option<String> {
    match name {
        TSTypeName::IdentifierReference(id) => Some(id.name.as_str().to_string()),
        TSTypeName::QualifiedName(qualified) => {
            let left = type_name_from_type_name(&qualified.left)?;
            let right = qualified.right.name.as_str();
            Some(format!("{}.{}", left, right))
        }
        TSTypeName::ThisExpression(_) => Some("this".to_string()),
    }
}

fn is_ctx_union(shapes: &[TypeShape]) -> bool {
    shapes.len() == 2
        && shapes.iter().all(|shape| {
            matches!(
                shape,
                TypeShape::Reference(reference)
                    if reference.ends_with("Ctx") || reference.ends_with("Context")
            )
        })
}

pub fn shape_signature(shape: &TypeShape) -> String {
    match shape {
        TypeShape::AnonymousObject(fields) => {
            let mut parts = fields
                .iter()
                .map(|field| {
                    format!(
                        "{}:{}{}",
                        field.name,
                        if field.optional { "?" } else { "" },
                        shape_signature(&field.shape)
                    )
                })
                .collect::<Vec<_>>();
            parts.sort();
            format!("{{{}}}", parts.join(","))
        }
        TypeShape::Primitive(value) | TypeShape::Literal(value) => value.clone(),
        TypeShape::Reference(value) => format!("@{}", value),
        TypeShape::Union(shapes) => {
            let mut parts = shapes.iter().map(shape_signature).collect::<Vec<_>>();
            parts.sort();
            format!("({})", parts.join("|"))
        }
        TypeShape::Intersection(shapes) => {
            let mut parts = shapes.iter().map(shape_signature).collect::<Vec<_>>();
            parts.sort();
            format!("[{}]", parts.join("&"))
        }
        TypeShape::Array(inner) => format!("{}[]", shape_signature(inner)),
        TypeShape::Tuple(items) => {
            let parts = items.iter().map(shape_signature).collect::<Vec<_>>();
            format!("[{}]", parts.join(","))
        }
        TypeShape::Function { params, returns } => {
            format!("fn({})=>{}", params.join(","), returns)
        }
        TypeShape::Other(value) => value.clone(),
    }
}

pub fn flatten_fields(fields: &[TypeField], prefix: &str) -> Vec<FieldBinding> {
    let mut out = Vec::new();
    for field in fields {
        let path = if prefix.is_empty() {
            field.name.clone()
        } else {
            format!("{}.{}", prefix, field.name)
        };
        let signature = shape_signature(&field.shape);
        let nested = match &field.shape {
            TypeShape::AnonymousObject(children) if !children.is_empty() => Some(NestedSlot {
                field_name: field.name.clone(),
                signature: signature.clone(),
                fields: flatten_fields(children, ""),
            }),
            _ => None,
        };
        out.push(FieldBinding {
            name: path.clone(),
            signature,
            optional: field.optional,
            nested,
        });
        if let TypeShape::AnonymousObject(children) = &field.shape {
            out.extend(flatten_fields(children, &path));
        }
    }
    out
}

pub fn top_level_bindings(def: &ParsedTypeDef) -> Vec<FieldBinding> {
    def.fields
        .iter()
        .map(|field| {
            let signature = shape_signature(&field.shape);
            let nested = match &field.shape {
                TypeShape::AnonymousObject(children) if !children.is_empty() => Some(NestedSlot {
                    field_name: field.name.clone(),
                    signature: signature.clone(),
                    fields: flatten_fields(children, ""),
                }),
                _ => None,
            };
            FieldBinding {
                name: field.name.clone(),
                signature,
                optional: field.optional,
                nested,
            }
        })
        .collect()
}

pub fn bundle_signature(bindings: &[FieldBinding]) -> String {
    let mut parts = bindings
        .iter()
        .map(|binding| format!("{}:{}", binding.name, binding.signature))
        .collect::<Vec<_>>();
    parts.sort();
    parts.join("|")
}

pub fn infer_bundle_name(bindings: &[FieldBinding]) -> String {
    let mut tokens = Vec::new();
    for binding in bindings {
        let base = binding
            .name
            .split('.')
            .next()
            .unwrap_or(&binding.name)
            .trim_end_matches("Id")
            .trim_end_matches("ID");
        let token = to_pascal_case(base);
        if !token.is_empty() && !tokens.contains(&token) {
            tokens.push(token);
        }
    }
    if tokens.is_empty() {
        "SharedFacts".to_string()
    } else if tokens.len() == 1 {
        format!("{}Facts", tokens[0])
    } else if tokens.len() <= 3 {
        format!("{}Facts", tokens.join(""))
    } else {
        format!("{}Facts", tokens[..3].join(""))
    }
}

fn to_pascal_case(value: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for ch in value.chars() {
        if ch == '_' || ch == '-' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn ts_literal_signature(literal: &oxc_ast::ast::TSLiteral<'_>) -> String {
    use oxc_ast::ast::TSLiteral;
    match literal {
        TSLiteral::BooleanLiteral(value) => value.value.to_string(),
        TSLiteral::NumericLiteral(value) => value.value.to_string(),
        TSLiteral::BigIntLiteral(value) => value
            .raw
            .as_ref()
            .map(|raw| raw.as_str().to_string())
            .unwrap_or_else(|| "bigint".to_string()),
        TSLiteral::StringLiteral(value) => value.value.to_string(),
        TSLiteral::TemplateLiteral(_) => "template".to_string(),
        TSLiteral::UnaryExpression(value) => format!("unary:{}", value.operator.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_interface_with_nested_object() {
        let source = r#"
interface TransferClaimNotifyArgs {
  raceId: Id<"races">;
  raceName: string;
  raceSlug: string;
  entrantId: Id<"entrants">;
  entrantName: string;
  claimToken: string;
  claimExpiresAt: number;
  recipientEmail: string;
}
"#;
        let parsed = parse_type_definition(source, "TransferClaimNotifyArgs", SymbolKind::Interface)
            .expect("parse interface");
        assert_eq!(parsed.fields.len(), 8);
        let nested = parsed
            .fields
            .iter()
            .find(|field| field.name == "raceId")
            .expect("raceId field");
        assert!(matches!(nested.shape, TypeShape::Reference(_)));
    }

    #[test]
    fn detects_ctx_union_boilerplate() {
        let source = "type MailCtx = MutationCtx | ActionCtx;";
        let parsed = parse_type_definition(source, "MailCtx", SymbolKind::TypeAlias).unwrap();
        assert!(parsed.is_boilerplate);
        assert!(!parsed.is_object_like);
    }

    #[test]
    fn nested_slots_share_signature() {
        let a = parse_type_definition(
            "interface A { race: { name: string; slug: string }; extra: string; }",
            "A",
            SymbolKind::Interface,
        )
        .unwrap();
        let b = parse_type_definition(
            "interface B { race: { slug: string; name: string }; other: number; }",
            "B",
            SymbolKind::Interface,
        )
        .unwrap();
        let slot_a = top_level_bindings(&a)[0]
            .nested
            .as_ref()
            .unwrap()
            .signature
            .clone();
        let slot_b = top_level_bindings(&b)[0]
            .nested
            .as_ref()
            .unwrap()
            .signature
            .clone();
        assert_eq!(slot_a, slot_b);
    }
}
