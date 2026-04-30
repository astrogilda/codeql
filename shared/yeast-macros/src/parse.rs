use proc_macro2::{Delimiter, Ident, Literal, Span, TokenStream, TokenTree};
use quote::quote;
use std::iter::Peekable;

type Tokens = Peekable<proc_macro2::token_stream::IntoIter>;
type Result<T> = std::result::Result<T, syn::Error>;

// ---------------------------------------------------------------------------
// Query parsing
// ---------------------------------------------------------------------------

/// Top-level entry: parse a single query node from the full input.
pub fn parse_query_top(input: TokenStream) -> Result<TokenStream> {
    let mut tokens = input.into_iter().peekable();
    let result = parse_query_node(&mut tokens)?;
    if let Some(tok) = tokens.next() {
        return Err(syn::Error::new_spanned(tok, "unexpected token after query"));
    }
    Ok(result)
}

/// Parse a single query node (possibly with a trailing `@capture`).
fn parse_query_node(tokens: &mut Tokens) -> Result<TokenStream> {
    let base = parse_query_atom(tokens)?;
    // Check for trailing @capture
    if peek_is_at(tokens) {
        tokens.next(); // consume @
        let capture_name = expect_ident(tokens, "expected capture name after @")?;
        let name_str = capture_name.to_string();
        Ok(quote! {
            yeast::query::QueryNode::Capture {
                capture: #name_str,
                node: Box::new(#base),
            }
        })
    } else {
        Ok(base)
    }
}

/// Parse a query atom: `(kind fields...)` or `(kind fields... bare_children...)`.
/// Does not handle `@capture` — that's handled by the caller as a postfix.
fn parse_query_atom(tokens: &mut Tokens) -> Result<TokenStream> {
    match tokens.peek() {
        None => Err(syn::Error::new(Span::call_site(), "unexpected end of query")),
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            let group = expect_group(tokens, Delimiter::Parenthesis)?;
            let mut inner = group.stream().into_iter().peekable();
            parse_query_node_inner(&mut inner)
        }
        Some(tok) => Err(syn::Error::new_spanned(
            tok.clone(),
            "expected `(` in query; use `(_) @name` to capture a wildcard",
        )),
    }
}

/// Parse the inside of a parenthesized query node: `kind fields...` or `_` or `"lit"`.
fn parse_query_node_inner(tokens: &mut Tokens) -> Result<TokenStream> {
    match tokens.peek() {
        None => Err(syn::Error::new(Span::call_site(), "empty parenthesized group in query")),
        Some(TokenTree::Ident(id)) if id.to_string() == "_" => {
            tokens.next();
            Ok(quote! { yeast::query::QueryNode::Any() })
        }
        Some(TokenTree::Literal(_)) => {
            let lit = expect_literal(tokens)?;
            Ok(quote! { yeast::query::QueryNode::UnnamedNode { kind: #lit } })
        }
        Some(TokenTree::Ident(_)) => {
            let kind = expect_ident(tokens, "expected node kind")?;
            let kind_str = kind.to_string();
            let fields = parse_query_fields(tokens)?;
            Ok(quote! {
                yeast::query::QueryNode::Node {
                    kind: #kind_str,
                    children: vec![#(#fields),*],
                }
            })
        }
        Some(tok) => Err(syn::Error::new_spanned(
            tok.clone(),
            "expected node kind, `_`, or string literal",
        )),
    }
}

/// Parse zero or more field specifications and trailing bare patterns.
/// Named fields: `name: pattern` or `name*: (list...)`.
/// Bare patterns (no field name) become implicit `child` field entries.
fn parse_query_fields(tokens: &mut Tokens) -> Result<Vec<TokenStream>> {
    let mut fields = Vec::new();
    while tokens.peek().is_some() {
        // Try to parse a named field: `ident :` or `ident * :`
        if peek_is_field(tokens) {
            let field_name = expect_ident(tokens, "expected field name")?;
            let field_str = field_name.to_string();

            let is_list = peek_is_star(tokens);
            if is_list {
                tokens.next(); // consume *
            }

            expect_punct(tokens, ':', "expected `:` after field name")?;

            if is_list {
                let group = expect_group(tokens, Delimiter::Parenthesis)?;
                let mut inner = group.stream().into_iter().peekable();
                let elems = parse_query_list(&mut inner)?;
                fields.push(quote! {
                    (#field_str, vec![#(#elems),*])
                });
            } else {
                let child = parse_query_node(tokens)?;
                fields.push(quote! {
                    (#field_str, vec![yeast::query::QueryListElem::SingleNode(#child)])
                });
            }
        } else {
            // Bare patterns — collect as implicit `child` field
            let elems = parse_query_list(tokens)?;
            if !elems.is_empty() {
                fields.push(quote! {
                    ("child", vec![#(#elems),*])
                });
            }
            break;
        }
    }
    Ok(fields)
}

/// Parse a list of query elements (inside a `child*:` field).
/// Each element is a node pattern, possibly followed by `*`, `+`, `?`.
fn parse_query_list(tokens: &mut Tokens) -> Result<Vec<TokenStream>> {
    let mut elems = Vec::new();
    while tokens.peek().is_some() {
        // Check for parenthesized group
        if peek_is_group(tokens, Delimiter::Parenthesis) {
            let group = expect_group(tokens, Delimiter::Parenthesis)?;
            let mut inner = group.stream().into_iter().peekable();

            // Check for repetition after the group
            if peek_is_repetition(tokens) {
                let rep = expect_repetition(tokens)?;
                // Determine if the group is a single node pattern or a list
                // of patterns. If it starts with an identifier (node kind) or
                // `_`, treat it as a single repeated node. Otherwise, parse
                // as a repeated list of sub-patterns.
                let is_single_node = matches!(inner.peek(), Some(TokenTree::Ident(_)));
                if is_single_node {
                    let node = parse_query_node_inner(&mut inner)?;
                    let elem = quote! {
                        yeast::query::QueryListElem::Repeated {
                            children: vec![yeast::query::QueryListElem::SingleNode(#node)],
                            rep: #rep,
                        }
                    };
                    let elem = maybe_wrap_list_capture(tokens, elem)?;
                    elems.push(elem);
                } else {
                    let sub_elems = parse_query_list(&mut inner)?;
                    let elem = quote! {
                        yeast::query::QueryListElem::Repeated {
                            children: vec![#(#sub_elems),*],
                            rep: #rep,
                        }
                    };
                    let elem = maybe_wrap_list_capture(tokens, elem)?;
                    elems.push(elem);
                }
            } else {
                // Single parenthesized node, possibly followed by @capture
                let node = parse_query_node_inner(&mut inner)?;
                let node = maybe_wrap_capture(tokens, node)?;
                elems.push(quote! {
                    yeast::query::QueryListElem::SingleNode(#node)
                });
            }
            continue;
        }

        // Check for string literal (unnamed node)
        if peek_is_literal(tokens) {
            let lit = expect_literal(tokens)?;
            let node = quote! { yeast::query::QueryNode::UnnamedNode { kind: #lit } };
            let elem = maybe_wrap_repetition(tokens, quote! {
                yeast::query::QueryListElem::SingleNode(#node)
            })?;
            elems.push(elem);
            continue;
        }

        // Check for bare _ (wildcard), possibly followed by @capture
        if peek_is_underscore(tokens) {
            tokens.next();
            let node = quote! { yeast::query::QueryNode::Any() };
            let node = maybe_wrap_capture(tokens, node)?;
            let elem = maybe_wrap_repetition(tokens, quote! {
                yeast::query::QueryListElem::SingleNode(#node)
            })?;
            elems.push(elem);
            continue;
        }

        break;
    }
    Ok(elems)
}

// ---------------------------------------------------------------------------
// Tree builder parsing
// ---------------------------------------------------------------------------

pub fn parse_tree_builder_top(input: TokenStream) -> Result<TokenStream> {
    let mut tokens = input.into_iter().peekable();
    let result = parse_builder_node(&mut tokens)?;
    if let Some(tok) = tokens.next() {
        return Err(syn::Error::new_spanned(tok, "unexpected token after tree_builder"));
    }
    Ok(result)
}

pub fn parse_trees_builder_top(input: TokenStream) -> Result<TokenStream> {
    let mut tokens = input.into_iter().peekable();
    if tokens.peek().is_none() {
        return Ok(quote! {
            yeast::tree_builder::TreesBuilder { children: Vec::new() }
        });
    }
    let children = parse_builder_child_list(&mut tokens)?;
    if let Some(tok) = tokens.next() {
        return Err(syn::Error::new_spanned(tok, "unexpected token after trees_builder"));
    }
    Ok(quote! {
        yeast::tree_builder::TreesBuilder {
            children: vec![#(#children),*],
        }
    })
}

/// Parse a single tree builder node.
fn parse_builder_node(tokens: &mut Tokens) -> Result<TokenStream> {
    match tokens.peek() {
        Some(TokenTree::Punct(p)) if p.as_char() == '@' => {
            tokens.next();
            let name = expect_ident(tokens, "expected capture name after @")?;
            let name_str = name.to_string();
            Ok(quote! {
                yeast::tree_builder::TreeBuilder::Capture { capture: #name_str }
            })
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            let group = expect_group(tokens, Delimiter::Parenthesis)?;
            let mut inner = group.stream().into_iter().peekable();
            parse_builder_node_inner(&mut inner)
        }
        Some(tok) => Err(syn::Error::new_spanned(tok.clone(), "expected `(` or `@` in tree_builder")),
        None => Err(syn::Error::new(Span::call_site(), "unexpected end of tree_builder")),
    }
}

/// Parse inside a parenthesized builder node.
fn parse_builder_node_inner(tokens: &mut Tokens) -> Result<TokenStream> {
    match tokens.peek() {
        Some(TokenTree::Ident(_)) => {
            let kind = expect_ident(tokens, "expected node kind")?;
            let kind_str = kind.to_string();

            // Check for (kind "literal") — Literal node
            if peek_is_literal(tokens) {
                let lit = expect_literal(tokens)?;
                return Ok(quote! {
                    yeast::tree_builder::TreeBuilder::Literal {
                        kind: #kind_str,
                        value: #lit,
                    }
                });
            }

            // Check for (kind $fresh) — Fresh node
            if peek_is_dollar(tokens) {
                tokens.next(); // consume $
                let name = expect_ident(tokens, "expected fresh variable name after $")?;
                let name_str = name.to_string();
                return Ok(quote! {
                    yeast::tree_builder::TreeBuilder::Fresh {
                        kind: #kind_str,
                        name: #name_str,
                    }
                });
            }

            let fields = parse_builder_fields(tokens)?;
            Ok(quote! {
                yeast::tree_builder::TreeBuilder::Node {
                    kind: #kind_str,
                    children: vec![#(#fields),*],
                }
            })
        }
        Some(TokenTree::Punct(p)) if p.as_char() == '@' => {
            tokens.next();
            let name = expect_ident(tokens, "expected capture name after @")?;
            let name_str = name.to_string();
            Ok(quote! {
                yeast::tree_builder::TreeBuilder::Capture { capture: #name_str }
            })
        }
        Some(tok) => Err(syn::Error::new_spanned(tok.clone(), "expected node kind or `@`")),
        None => Err(syn::Error::new(Span::call_site(), "empty builder group")),
    }
}

/// Parse builder fields and trailing bare patterns (implicit `child` field).
fn parse_builder_fields(tokens: &mut Tokens) -> Result<Vec<TokenStream>> {
    let mut fields = Vec::new();
    while tokens.peek().is_some() {
        if peek_is_field(tokens) {
            let field_name = expect_ident(tokens, "expected field name")?;
            let field_str = field_name.to_string();

            let is_list = peek_is_star(tokens);
            if is_list {
                tokens.next();
            }

            expect_punct(tokens, ':', "expected `:` after field name")?;

            if is_list {
                let group = expect_group(tokens, Delimiter::Parenthesis)?;
                let mut inner = group.stream().into_iter().peekable();
                let children = parse_builder_child_list(&mut inner)?;
                fields.push(quote! {
                    (#field_str, vec![#(#children),*])
                });
            } else {
                let child = parse_builder_node(tokens)?;
                fields.push(quote! {
                    (#field_str, vec![yeast::tree_builder::TreeChildBuilder::SingleNode(#child)])
                });
            }
        } else {
            // Bare patterns — collect as implicit `child` field
            let children = parse_builder_child_list(tokens)?;
            if !children.is_empty() {
                fields.push(quote! {
                    ("child", vec![#(#children),*])
                });
            }
            break;
        }
    }
    Ok(fields)
}

/// Parse a list of builder children (for `child*:` or `trees_builder!` top level).
fn parse_builder_child_list(tokens: &mut Tokens) -> Result<Vec<TokenStream>> {
    let mut children = Vec::new();
    while tokens.peek().is_some() {
        if peek_is_at(tokens) {
            let node = parse_builder_node(tokens)?;
            // Check for * repetition
            if peek_is_star(tokens) {
                tokens.next();
                children.push(quote! {
                    yeast::tree_builder::TreeChildBuilder::Repeated { child: #node }
                });
            } else {
                children.push(quote! {
                    yeast::tree_builder::TreeChildBuilder::SingleNode(#node)
                });
            }
            continue;
        }

        if peek_is_group(tokens, Delimiter::Parenthesis) {
            let group = expect_group(tokens, Delimiter::Parenthesis)?;
            let mut inner = group.stream().into_iter().peekable();

            if peek_is_star(tokens) {
                // (pattern)* — repeated child
                tokens.next();
                let node = parse_builder_node_inner(&mut inner)?;
                children.push(quote! {
                    yeast::tree_builder::TreeChildBuilder::Repeated { child: #node }
                });
            } else {
                // (pattern) — single child
                let node = parse_builder_node_inner(&mut inner)?;
                children.push(quote! {
                    yeast::tree_builder::TreeChildBuilder::SingleNode(#node)
                });
            }
            continue;
        }

        break;
    }
    Ok(children)
}

// ---------------------------------------------------------------------------
// tree! / trees! parsing — direct code generation against BuildCtx
// ---------------------------------------------------------------------------

/// Parse `tree!(ctx, template)` — unified macro that returns `Id` for a single
/// top-level element or `Vec<Id>` for multiple elements.
pub fn parse_tree_top(input: TokenStream) -> Result<TokenStream> {
    let mut tokens = input.into_iter().peekable();
    let ctx = expect_ident(&mut tokens, "expected build context identifier")?;
    expect_punct(&mut tokens, ',', "expected `,` after context")?;

    // Parse the first element
    let first = parse_direct_node(&mut tokens, &ctx)?;

    // If nothing follows, return a single Id
    if tokens.peek().is_none() {
        return Ok(quote! { { #first } });
    }

    // Multiple elements — collect into Vec<Id>
    let mut items = vec![quote! { __nodes.push(#first); }];
    let rest = parse_direct_list(&mut tokens, &ctx)?;
    items.extend(rest);

    if let Some(tok) = tokens.next() {
        return Err(syn::Error::new_spanned(tok, "unexpected token after tree! template"));
    }

    Ok(quote! {
        {
            let mut __nodes: Vec<usize> = Vec::new();
            #(#items)*
            __nodes
        }
    })
}

/// Kept for backward compatibility — identical to `parse_tree_top`.
pub fn parse_trees_top(input: TokenStream) -> Result<TokenStream> {
    parse_tree_top(input)
}

/// Parse a single node template and generate code that returns an `Id`.
/// Handles: `(kind fields... children...)`, `@capture`, `{expr}`.
fn parse_direct_node(tokens: &mut Tokens, ctx: &Ident) -> Result<TokenStream> {
    match tokens.peek() {
        Some(TokenTree::Punct(p)) if p.as_char() == '@' => {
            tokens.next();
            let name = expect_ident(tokens, "expected capture name after @")?;
            let name_str = name.to_string();
            Ok(quote! { #ctx.capture(#name_str) })
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
            let group = expect_group(tokens, Delimiter::Brace)?;
            let expr = group.stream();
            Ok(quote! { #expr })
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            let group = expect_group(tokens, Delimiter::Parenthesis)?;
            let mut inner = group.stream().into_iter().peekable();
            parse_direct_node_inner(&mut inner, ctx)
        }
        Some(tok) => Err(syn::Error::new_spanned(tok.clone(), "expected `(`, `@`, or `{` in tree template")),
        None => Err(syn::Error::new(Span::call_site(), "unexpected end of tree template")),
    }
}

/// Parse the inside of a parenthesized node: `kind fields... children...`
/// or `kind "literal"` or `kind $fresh`.
fn parse_direct_node_inner(tokens: &mut Tokens, ctx: &Ident) -> Result<TokenStream> {
    let kind = expect_ident(tokens, "expected node kind")?;
    let kind_str = kind.to_string();

    // Check for (kind "literal")
    if peek_is_literal(tokens) {
        let lit = expect_literal(tokens)?;
        return Ok(quote! { #ctx.literal(#kind_str, #lit) });
    }

    // Check for (kind #{expr}) — computed literal, expr converted via .to_string()
    if peek_is_hash(tokens) {
        tokens.next(); // consume #
        let group = expect_group(tokens, Delimiter::Brace)?;
        let expr = group.stream();
        return Ok(quote! { #ctx.literal(#kind_str, &(#expr).to_string()) });
    }

    // Check for (kind $fresh)
    if peek_is_dollar(tokens) {
        tokens.next();
        let name = expect_ident(tokens, "expected fresh variable name after $")?;
        let name_str = name.to_string();
        return Ok(quote! { #ctx.fresh(#kind_str, #name_str) });
    }

    // Parse named fields and bare children
    let mut stmts = Vec::new();
    let mut field_args = Vec::new();
    let mut has_children = false;

    // Named fields — compute each value into a temp, then reference it
    while peek_is_field(tokens) {
        let field_name = expect_ident(tokens, "expected field name")?;
        let field_str = field_name.to_string();
        expect_punct(tokens, ':', "expected `:` after field name")?;
        let value = parse_direct_node(tokens, ctx)?;
        let temp = Ident::new(&format!("__field_{field_str}"), Span::call_site());
        stmts.push(quote! { let #temp = #value; });
        field_args.push(quote! { (#field_str, vec![#temp]) });
    }

    // Bare children (implicit "child" field)
    let mut child_stmts = Vec::new();
    while tokens.peek().is_some() {
        if peek_is_group(tokens, Delimiter::Parenthesis) {
            let group = expect_group(tokens, Delimiter::Parenthesis)?;
            let mut inner = group.stream().into_iter().peekable();

            if peek_is_star(tokens) {
                tokens.next();
                if peek_is_at(&mut inner) {
                    inner.next();
                    let name = expect_ident(&mut inner, "expected capture name")?;
                    let name_str = name.to_string();
                    child_stmts.push(quote! {
                        __children.extend(#ctx.capture_all(#name_str));
                    });
                } else {
                    let node = parse_direct_node_inner(&mut inner, ctx)?;
                    child_stmts.push(quote! { __children.push(#node); });
                }
                has_children = true;
                continue;
            }

            let node = parse_direct_node_inner(&mut inner, ctx)?;
            child_stmts.push(quote! { __children.push(#node); });
            has_children = true;
            continue;
        }

        if peek_is_at(tokens) {
            let node = parse_direct_node(tokens, ctx)?;
            child_stmts.push(quote! { __children.push(#node); });
            has_children = true;
            continue;
        }

        if peek_is_group(tokens, Delimiter::Brace) {
            let group = expect_group(tokens, Delimiter::Brace)?;
            let mut inner = group.stream().into_iter().peekable();
            if peek_is_dotdot(&mut inner) {
                inner.next();
                inner.next();
                let expr: TokenStream = inner.collect();
                child_stmts.push(quote! { __children.extend(#expr); });
            } else {
                let expr = group.stream();
                child_stmts.push(quote! { __children.push(#expr); });
            }
            has_children = true;
            continue;
        }

        break;
    }

    if !has_children {
        Ok(quote! {
            {
                #(#stmts)*
                #ctx.node(#kind_str, vec![#(#field_args),*])
            }
        })
    } else {
        Ok(quote! {
            {
                #(#stmts)*
                let mut __children: Vec<usize> = Vec::new();
                #(#child_stmts)*
                #ctx.with_children(#kind_str, vec![#(#field_args),*], __children)
            }
        })
    }
}

/// Parse the top-level list of a `trees!` template.
/// Each item is a node template, `(@capture)*` splice, or `{expr}` splice.
fn parse_direct_list(tokens: &mut Tokens, ctx: &Ident) -> Result<Vec<TokenStream>> {
    let mut items = Vec::new();
    while tokens.peek().is_some() {
        // (@name)* — splice repeated capture
        if peek_is_group(tokens, Delimiter::Parenthesis) {
            let group = expect_group(tokens, Delimiter::Parenthesis)?;
            let mut inner = group.stream().into_iter().peekable();

            if peek_is_star(tokens) {
                tokens.next();
                // Inside parens should be @name
                if peek_is_at(&mut inner) {
                    inner.next();
                    let name = expect_ident(&mut inner, "expected capture name")?;
                    let name_str = name.to_string();
                    items.push(quote! {
                        __nodes.extend(#ctx.capture_all(#name_str));
                    });
                } else {
                    return Err(syn::Error::new(Span::call_site(), "expected @capture inside (...)* splice"));
                }
                continue;
            }

            // Regular node
            let node = parse_direct_node_inner(&mut inner, ctx)?;
            items.push(quote! { __nodes.push(#node); });
            continue;
        }

        // {expr} or {..expr} — single node or splice
        if peek_is_group(tokens, Delimiter::Brace) {
            let group = expect_group(tokens, Delimiter::Brace)?;
            let mut inner = group.stream().into_iter().peekable();
            if peek_is_dotdot(&mut inner) {
                inner.next(); // consume first .
                inner.next(); // consume second .
                let expr: TokenStream = inner.collect();
                items.push(quote! { __nodes.extend(#expr); });
            } else {
                let expr = group.stream();
                items.push(quote! { __nodes.push(#expr); });
            }
            continue;
        }

        // @capture — single capture
        if peek_is_at(tokens) {
            let node = parse_direct_node(tokens, ctx)?;
            items.push(quote! { __nodes.push(#node); });
            continue;
        }

        break;
    }
    Ok(items)
}

// ---------------------------------------------------------------------------
// Token utilities
// ---------------------------------------------------------------------------

fn peek_is_at(tokens: &mut Tokens) -> bool {
    matches!(tokens.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '@')
}

fn peek_is_star(tokens: &mut Tokens) -> bool {
    matches!(tokens.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '*')
}

fn peek_is_literal(tokens: &mut Tokens) -> bool {
    matches!(tokens.peek(), Some(TokenTree::Literal(_)))
}

fn peek_is_dollar(tokens: &mut Tokens) -> bool {
    matches!(tokens.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '$')
}

fn peek_is_hash(tokens: &mut Tokens) -> bool {
    matches!(tokens.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '#')
}

/// Check for `..` (two consecutive dot punctuation tokens).
fn peek_is_dotdot(tokens: &Tokens) -> bool {
    let mut lookahead = tokens.clone();
    matches!(lookahead.next(), Some(TokenTree::Punct(p)) if p.as_char() == '.')
        && matches!(lookahead.next(), Some(TokenTree::Punct(p)) if p.as_char() == '.')
}

fn peek_is_underscore(tokens: &mut Tokens) -> bool {
    matches!(tokens.peek(), Some(TokenTree::Ident(id)) if id.to_string() == "_")
}

/// Check if the next tokens form a field specification (ident followed by `:` or `*:`).
/// A bare identifier (other than `_`) at this position is always a field name, since
/// bare child patterns must start with `(`, `@`, `"literal"`, or `_`.
fn peek_is_field(tokens: &mut Tokens) -> bool {
    match tokens.peek() {
        Some(TokenTree::Ident(id)) if id.to_string() != "_" => true,
        _ => false,
    }
}

fn peek_is_group(tokens: &mut Tokens, delim: Delimiter) -> bool {
    matches!(tokens.peek(), Some(TokenTree::Group(g)) if g.delimiter() == delim)
}

fn peek_is_repetition(tokens: &mut Tokens) -> bool {
    matches!(tokens.peek(), Some(TokenTree::Punct(p)) if matches!(p.as_char(), '*' | '+' | '?'))
}

fn expect_ident(tokens: &mut Tokens, msg: &str) -> Result<Ident> {
    match tokens.next() {
        Some(TokenTree::Ident(id)) => Ok(id),
        Some(tok) => Err(syn::Error::new_spanned(tok, msg)),
        None => Err(syn::Error::new(Span::call_site(), msg)),
    }
}

fn expect_literal(tokens: &mut Tokens) -> Result<Literal> {
    match tokens.next() {
        Some(TokenTree::Literal(lit)) => Ok(lit),
        Some(tok) => Err(syn::Error::new_spanned(tok, "expected string literal")),
        None => Err(syn::Error::new(Span::call_site(), "expected string literal")),
    }
}

fn expect_punct(tokens: &mut Tokens, ch: char, msg: &str) -> Result<()> {
    match tokens.next() {
        Some(TokenTree::Punct(p)) if p.as_char() == ch => Ok(()),
        Some(tok) => Err(syn::Error::new_spanned(tok, msg)),
        None => Err(syn::Error::new(Span::call_site(), msg)),
    }
}

fn expect_group(tokens: &mut Tokens, delim: Delimiter) -> Result<proc_macro2::Group> {
    match tokens.next() {
        Some(TokenTree::Group(g)) if g.delimiter() == delim => Ok(g),
        Some(tok) => Err(syn::Error::new_spanned(
            tok,
            format!("expected {:?} group", delim),
        )),
        None => Err(syn::Error::new(
            Span::call_site(),
            format!("expected {:?} group", delim),
        )),
    }
}

fn expect_repetition(tokens: &mut Tokens) -> Result<TokenStream> {
    match tokens.next() {
        Some(TokenTree::Punct(p)) => match p.as_char() {
            '*' => Ok(quote! { yeast::query::Rep::ZeroOrMore }),
            '+' => Ok(quote! { yeast::query::Rep::OneOrMore }),
            '?' => Ok(quote! { yeast::query::Rep::ZeroOrOne }),
            _ => Err(syn::Error::new(p.span(), "expected `*`, `+`, or `?`")),
        },
        Some(tok) => Err(syn::Error::new_spanned(tok, "expected repetition quantifier")),
        None => Err(syn::Error::new(Span::call_site(), "expected repetition quantifier")),
    }
}

fn maybe_wrap_capture(tokens: &mut Tokens, base: TokenStream) -> Result<TokenStream> {
    if peek_is_at(tokens) {
        tokens.next(); // consume @
        let name = expect_ident(tokens, "expected capture name after @")?;
        let name_str = name.to_string();
        Ok(quote! {
            yeast::query::QueryNode::Capture {
                capture: #name_str,
                node: Box::new(#base),
            }
        })
    } else {
        Ok(base)
    }
}

fn maybe_wrap_repetition(tokens: &mut Tokens, single: TokenStream) -> Result<TokenStream> {
    if peek_is_repetition(tokens) {
        let rep = expect_repetition(tokens)?;
        Ok(quote! {
            yeast::query::QueryListElem::Repeated {
                children: vec![#single],
                rep: #rep,
            }
        })
    } else {
        Ok(single)
    }
}

/// If `@name` follows a Repeated list element, wrap each child SingleNode
/// inside the repetition with a Capture. This matches tree-sitter semantics
/// where `(_)* @name` captures each matched node.
fn maybe_wrap_list_capture(tokens: &mut Tokens, elem: TokenStream) -> Result<TokenStream> {
    if peek_is_at(tokens) {
        tokens.next();
        let name = expect_ident(tokens, "expected capture name after @")?;
        let name_str = name.to_string();
        // Re-parse the element isn't practical, so we generate a wrapper
        // that creates a new Repeated with each child wrapped in a capture.
        // The simplest approach: generate code that the runtime can interpret.
        // Actually, the capture annotation on repeated elements is best handled
        // by re-generating the Repeated with captures injected.
        // For now, assume the common case: the repetition contains a single
        // SingleNode child, and we wrap that node in a capture.
        Ok(quote! {
            {
                let __rep = #elem;
                match __rep {
                    yeast::query::QueryListElem::Repeated { children, rep } => {
                        yeast::query::QueryListElem::Repeated {
                            children: children.into_iter().map(|child| {
                                match child {
                                    yeast::query::QueryListElem::SingleNode(node) => {
                                        yeast::query::QueryListElem::SingleNode(
                                            yeast::query::QueryNode::Capture {
                                                capture: #name_str,
                                                node: Box::new(node),
                                            }
                                        )
                                    }
                                    other => other,
                                }
                            }).collect(),
                            rep,
                        }
                    }
                    other => other,
                }
            }
        })
    } else {
        Ok(elem)
    }
}
