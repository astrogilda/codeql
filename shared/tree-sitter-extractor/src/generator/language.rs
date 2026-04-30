pub struct Language {
    pub name: String,
    pub node_types: &'static str,
    /// If set, the generator uses these node types for the dbscheme/QL library
    /// instead of `node_types`. This is useful when desugaring transforms produce
    /// an AST whose shape differs from the tree-sitter grammar.
    pub output_node_types: Option<&'static str>,
}
