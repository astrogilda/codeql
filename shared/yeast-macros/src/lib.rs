use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;

mod parse;

/// Proc macro for constructing a `QueryNode` from a tree-sitter-inspired pattern.
///
/// # Syntax
///
/// ```text
/// (_)                          - match any node
/// (kind)                       - match a named node of the given kind
/// ("literal")                  - match an unnamed node (e.g. operators, keywords)
/// (kind field: (pattern))      - match with named field
/// (kind child*: (patterns...)) - match unnamed children (yeast-specific)
/// (pattern) @capture           - capture the matched node
/// @capture                     - capture any node (shorthand for (_) @capture)
/// (pat1 pat2)*                 - zero or more repetitions
/// (pat1 pat2)+                 - one or more repetitions
/// (pat1 pat2)?                 - zero or one repetitions
/// ```
#[proc_macro]
pub fn query(input: TokenStream) -> TokenStream {
    let input2: TokenStream2 = input.into();
    match parse::parse_query_top(input2) {
        Ok(output) => output.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Build a single AST node from a template, returning its `Id`.
///
/// # Syntax
///
/// ```text
/// tree!(ctx,
///     (kind
///         field: (child_kind ...)
///         field: @capture
///         (identifier "literal_value")
///         (identifier $fresh_name)
///         {rust_expression}
///     )
/// )
/// ```
///
/// `ctx` must be a `&mut BuildCtx`.
#[proc_macro]
pub fn tree(input: TokenStream) -> TokenStream {
    let input2: TokenStream2 = input.into();
    match parse::parse_tree_top(input2) {
        Ok(output) => output.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Build a list of AST nodes from a template, returning `Vec<Id>`.
///
/// # Syntax
///
/// ```text
/// trees!(ctx,
///     (node1 ...)
///     (node2 ...)
///     (@capture)*          // splice repeated capture
///     {rust_expression}    // splice Vec<Id>
/// )
/// ```
///
/// `ctx` must be a `&mut BuildCtx`.
#[proc_macro]
pub fn trees(input: TokenStream) -> TokenStream {
    let input2: TokenStream2 = input.into();
    match parse::parse_trees_top(input2) {
        Ok(output) => output.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
