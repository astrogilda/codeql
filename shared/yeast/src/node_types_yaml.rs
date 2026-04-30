/// Converts a YAML node-types file to the tree-sitter `node-types.json` format.
///
/// # YAML format
///
/// ```yaml
/// supertypes:
///   _expression:
///     - assignment
///     - binary
///
/// named:
///   assignment:
///     left: _lhs
///     right: _expression
///   identifier:
///
/// unnamed:
///   - "+"
///   - "end"
/// ```
///
/// See the crate-level docs for the full format specification.
use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::json;

/// Top-level YAML structure.
#[derive(Deserialize, Default)]
struct YamlNodeTypes {
    #[serde(default)]
    supertypes: BTreeMap<String, Vec<TypeRef>>,
    #[serde(default)]
    named: BTreeMap<String, Option<BTreeMap<String, TypeRefOrList>>>,
    #[serde(default)]
    unnamed: Vec<String>,
}

/// A reference to a node type. Can be:
/// - a plain string (resolved by looking up named vs unnamed)
/// - a map `{unnamed: "name"}` to force unnamed interpretation
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum TypeRef {
    Name(String),
    Explicit { unnamed: String },
}

/// A field value: either a single type ref or a list of them.
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum TypeRefOrList {
    Single(TypeRef),
    List(Vec<TypeRef>),
}

impl TypeRefOrList {
    fn into_vec(self) -> Vec<TypeRef> {
        match self {
            TypeRefOrList::Single(t) => vec![t],
            TypeRefOrList::List(v) => v,
        }
    }
}

/// Parsed field name: base name + multiplicity markers.
struct FieldSpec {
    name: Option<String>, // None for $children
    multiple: bool,
    required: bool,
}

fn parse_field_name(raw: &str) -> FieldSpec {
    let is_children = raw.starts_with("$children");

    let suffix = raw.chars().last().filter(|c| matches!(c, '?' | '*' | '+'));

    let (multiple, required) = match suffix {
        Some('?') => (false, false),
        Some('*') => (true, false),
        Some('+') => (true, true),
        _ => (false, true), // bare field name = required, single
    };

    let name = if is_children {
        None
    } else {
        let base = raw.trim_end_matches(|c: char| matches!(c, '?' | '*' | '+'));
        Some(base.to_string())
    };

    FieldSpec {
        name,
        multiple,
        required,
    }
}

/// Resolve a TypeRef to a (type, named) pair, given the sets of known named
/// and unnamed types.
fn resolve_type_ref(
    type_ref: &TypeRef,
    named_types: &BTreeSet<String>,
    unnamed_types: &BTreeSet<String>,
) -> serde_json::Value {
    match type_ref {
        TypeRef::Explicit { unnamed } => {
            json!({"type": unnamed, "named": false})
        }
        TypeRef::Name(name) => {
            let is_named = named_types.contains(name);
            let is_unnamed = unnamed_types.contains(name);

            if is_named && is_unnamed {
                // Ambiguous: default to named
                json!({"type": name, "named": true})
            } else if is_unnamed {
                json!({"type": name, "named": false})
            } else {
                // Named, or unknown (assume named)
                json!({"type": name, "named": true})
            }
        }
    }
}

/// Convert YAML string to node-types JSON string.
pub fn convert(yaml_input: &str) -> Result<String, String> {
    let yaml: YamlNodeTypes =
        serde_yaml::from_str(yaml_input).map_err(|e| format!("Failed to parse YAML: {e}"))?;

    // Build the sets of known named and unnamed types for resolution.
    let mut named_types = BTreeSet::new();
    for name in yaml.supertypes.keys() {
        named_types.insert(name.clone());
    }
    for name in yaml.named.keys() {
        named_types.insert(name.clone());
    }
    let unnamed_types: BTreeSet<String> = yaml.unnamed.iter().cloned().collect();

    let mut output = Vec::new();

    // 1. Supertypes
    for (name, members) in &yaml.supertypes {
        let subtypes: Vec<_> = members
            .iter()
            .map(|m| resolve_type_ref(m, &named_types, &unnamed_types))
            .collect();
        output.push(json!({
            "type": name,
            "named": true,
            "subtypes": subtypes,
        }));
    }

    // 2. Named nodes
    for (name, fields_opt) in &yaml.named {
        let fields_map = match fields_opt {
            None => {
                // Leaf token: no fields, no children, no subtypes
                output.push(json!({
                    "type": name,
                    "named": true,
                    "fields": {},
                }));
                continue;
            }
            Some(m) if m.is_empty() => {
                output.push(json!({
                    "type": name,
                    "named": true,
                    "fields": {},
                }));
                continue;
            }
            Some(m) => m,
        };

        let mut json_fields = serde_json::Map::new();
        let mut json_children: Option<serde_json::Value> = None;

        for (raw_field_name, type_refs) in fields_map {
            let spec = parse_field_name(raw_field_name);
            let types: Vec<_> = type_refs
                .clone()
                .into_vec()
                .iter()
                .map(|t| resolve_type_ref(t, &named_types, &unnamed_types))
                .collect();

            // Cloning to make the borrow checker happy
            let field_info = json!({
                "multiple": spec.multiple,
                "required": spec.required,
                "types": types,
            });

            if spec.name.is_none() {
                // $children
                json_children = Some(field_info);
            } else {
                json_fields.insert(spec.name.unwrap(), field_info);
            }
        }

        let mut entry = json!({
            "type": name,
            "named": true,
            "fields": json_fields,
        });

        if let Some(children) = json_children {
            entry
                .as_object_mut()
                .unwrap()
                .insert("children".to_string(), children);
        }

        output.push(entry);
    }

    // 3. Unnamed tokens
    for name in &yaml.unnamed {
        output.push(json!({
            "type": name,
            "named": false,
        }));
    }

    serde_json::to_string_pretty(&output).map_err(|e| format!("Failed to serialize JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_conversion() {
        let yaml = r#"
supertypes:
  _expression:
    - assignment
    - binary

named:
  assignment:
    left: _lhs
    right: _expression
  binary:
    left: [_expression, _simple_numeric]
    operator: ["!=", "+"]
    right: _expression
  argument_list:
    $children*: [_expression, block_argument]
  identifier:

unnamed:
  - "!="
  - "+"
  - "end"
"#;

        let json_str = convert(yaml).unwrap();
        let result: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap();

        // Check supertype
        let expr = &result[0];
        assert_eq!(expr["type"], "_expression");
        assert_eq!(expr["named"], true);
        assert_eq!(expr["subtypes"].as_array().unwrap().len(), 2);

        // Check assignment
        let assign = result.iter().find(|n| n["type"] == "assignment").unwrap();
        assert_eq!(assign["fields"]["left"]["required"], true);
        assert_eq!(assign["fields"]["left"]["multiple"], false);
        assert_eq!(assign["fields"]["left"]["types"][0]["type"], "_lhs");
        assert_eq!(assign["fields"]["left"]["types"][0]["named"], true);

        // Check binary.operator — "!=" and "+" should resolve to unnamed
        let binary = result.iter().find(|n| n["type"] == "binary").unwrap();
        let op_types = binary["fields"]["operator"]["types"].as_array().unwrap();
        assert_eq!(op_types[0]["type"], "!=");
        assert_eq!(op_types[0]["named"], false);
        assert_eq!(op_types[1]["type"], "+");
        assert_eq!(op_types[1]["named"], false);

        // Check argument_list has children, not a field
        let arg_list = result
            .iter()
            .find(|n| n["type"] == "argument_list")
            .unwrap();
        assert!(arg_list.get("children").is_some());
        assert_eq!(arg_list["children"]["multiple"], true);
        assert_eq!(arg_list["children"]["required"], false);

        // Check identifier is a leaf
        let ident = result.iter().find(|n| n["type"] == "identifier").unwrap();
        assert_eq!(ident["fields"].as_object().unwrap().len(), 0);

        // Check unnamed tokens
        let end = result.iter().find(|n| n["type"] == "end").unwrap();
        assert_eq!(end["named"], false);
    }

    #[test]
    fn test_explicit_unnamed_disambiguation() {
        let yaml = r#"
named:
  foo:
    field: [{unnamed: bar}]

unnamed:
  - bar
"#;

        let json_str = convert(yaml).unwrap();
        let result: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap();
        let foo = result.iter().find(|n| n["type"] == "foo").unwrap();
        assert_eq!(foo["fields"]["field"]["types"][0]["named"], false);
    }

    #[test]
    fn test_field_suffixes() {
        let yaml = r#"
named:
  test_node:
    required_single: foo
    optional_single?: foo
    required_multiple+: foo
    optional_multiple*: foo
"#;

        let json_str = convert(yaml).unwrap();
        let result: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap();
        let node = result.iter().find(|n| n["type"] == "test_node").unwrap();
        let fields = node["fields"].as_object().unwrap();

        assert_eq!(fields["required_single"]["required"], true);
        assert_eq!(fields["required_single"]["multiple"], false);

        assert_eq!(fields["optional_single"]["required"], false);
        assert_eq!(fields["optional_single"]["multiple"], false);

        assert_eq!(fields["required_multiple"]["required"], true);
        assert_eq!(fields["required_multiple"]["multiple"], true);

        assert_eq!(fields["optional_multiple"]["required"], false);
        assert_eq!(fields["optional_multiple"]["multiple"], true);
    }
}
