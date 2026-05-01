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
/// Can be called with an explicit context or using the implicit context
/// from an enclosing `rule!`:
///
/// ```text
/// tree!(ctx, (kind ...))     // explicit BuildCtx
/// tree!((kind ...))          // implicit context from rule!
/// ```
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
/// Can be called with an explicit context or using the implicit context
/// from an enclosing `rule!`:
///
/// ```text
/// trees!(ctx, (node1 ...) (node2 ...))   // explicit BuildCtx
/// trees!((node1 ...) (node2 ...))        // implicit context from rule!
/// ```
#[proc_macro]
pub fn trees(input: TokenStream) -> TokenStream {
    let input2: TokenStream2 = input.into();
    match parse::parse_trees_top(input2) {
        Ok(output) => output.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Define a desugaring rule with query and transform in one declaration.
///
/// ```text
/// rule!(
///     (query_pattern
///         field: (_) @capture_name
///         (kind)* @repeated
///     )
///     =>
///     (output_template
///         field: {capture_name}
///         {..repeated}
///     )
/// )
/// ```
///
/// Captures become Rust variables: `@name` binds `name: Id` (single)
/// or `name: Vec<Id>` (after `*`/`+`). The transform can use `tree!`
/// and `trees!` without an explicit context.
#[proc_macro]
pub fn rule(input: TokenStream) -> TokenStream {
    let input2: TokenStream2 = input.into();
    match parse::parse_rule_top(input2) {
        Ok(output) => output.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
