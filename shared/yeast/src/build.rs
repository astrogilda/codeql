use std::collections::BTreeMap;

use crate::captures::Captures;
use crate::tree_builder::FreshScope;
use crate::{Ast, Id, NodeContent, CHILD_FIELD};

/// Context for building new AST nodes during a transformation.
///
/// Used by the `tree!` and `trees!` macros. Holds a mutable reference to the
/// AST, a reference to the captures from a query match, and a `FreshScope` for
/// generating unique identifiers.
pub struct BuildCtx<'a> {
    pub ast: &'a mut Ast,
    pub captures: &'a Captures,
    pub fresh: &'a FreshScope,
}

impl<'a> BuildCtx<'a> {
    pub fn new(ast: &'a mut Ast, captures: &'a Captures, fresh: &'a FreshScope) -> Self {
        Self {
            ast,
            captures,
            fresh,
        }
    }

    /// Look up a capture variable, returning its node Id.
    pub fn capture(&self, name: &str) -> Id {
        self.captures
            .get_var(name)
            .unwrap_or_else(|e| panic!("build: {e}"))
    }

    /// Get all values of a repeated capture variable.
    pub fn capture_all(&self, name: &str) -> Vec<Id> {
        self.captures.get_all(name)
    }

    /// Create a named AST node with the given kind and fields.
    pub fn node(&mut self, kind: &str, fields: Vec<(&str, Vec<Id>)>) -> Id {
        let kind_id = self
            .ast
            .id_for_node_kind(kind)
            .unwrap_or_else(|| panic!("build: node kind '{kind}' not found"));
        let field_map: BTreeMap<_, _> = fields
            .into_iter()
            .map(|(name, ids)| {
                let field_id = self
                    .ast
                    .field_id_for_name(name)
                    .unwrap_or_else(|| panic!("build: field '{name}' not found"));
                (field_id, ids)
            })
            .collect();
        self.ast
            .create_node(kind_id, NodeContent::DynamicString(String::new()), field_map, true)
    }

    /// Create a leaf node with a fixed string content.
    pub fn literal(&mut self, kind: &'static str, value: &str) -> Id {
        self.ast.create_named_token(kind, value.to_string())
    }

    /// Create a leaf node with an auto-generated unique name.
    pub fn fresh(&mut self, kind: &'static str, name: &str) -> Id {
        let generated = self.fresh.resolve(name);
        self.ast.create_named_token(kind, generated)
    }

    /// Create a node for unnamed children (the synthetic "child" field).
    pub fn with_children(&mut self, kind: &str, named_fields: Vec<(&str, Vec<Id>)>, children: Vec<Id>) -> Id {
        let kind_id = self
            .ast
            .id_for_node_kind(kind)
            .unwrap_or_else(|| panic!("build: node kind '{kind}' not found"));
        let mut field_map: BTreeMap<_, _> = named_fields
            .into_iter()
            .map(|(name, ids)| {
                let field_id = self
                    .ast
                    .field_id_for_name(name)
                    .unwrap_or_else(|| panic!("build: field '{name}' not found"));
                (field_id, ids)
            })
            .collect();
        if !children.is_empty() {
            field_map.insert(CHILD_FIELD, children);
        }
        self.ast
            .create_node(kind_id, NodeContent::DynamicString(String::new()), field_map, true)
    }
}
