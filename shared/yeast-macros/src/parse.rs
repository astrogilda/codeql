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

/// Parse a query atom: `_`, `"literal"`, `@capture`, or `(kind fields...)`.
fn parse_query_atom(tokens: &mut Tokens) -> Result<TokenStream> {
    match tokens.peek() {
        None => Err(syn::Error::new(Span::call_site(), "unexpected end of query")),
        Some(TokenTree::Punct(p)) if p.as_char() == '@' => {
            // @capture shorthand (implicit wildcard)
            tokens.next(); // consume @
            let capture_name = expect_ident(tokens, "expected capture name after @")?;
            let name_str = capture_name.to_string();
            Ok(quote! {
                yeast::query::QueryNode::Capture {
                    capture: #name_str,
                    node: Box::new(yeast::query::QueryNode::Any()),
                }
            })
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            let group = expect_group(tokens, Delimiter::Parenthesis)?;
            let mut inner = group.stream().into_iter().peekable();
            parse_query_node_inner(&mut inner)
        }
        Some(tok) => Err(syn::Error::new_spanned(
            tok.clone(),
            "expected `(`, `@`, or `_` in query",
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
        Some(TokenTree::Punct(p)) if p.as_char() == '@' => {
            // (@capture) inside parens — just delegate
            let node = parse_query_atom(tokens)?;
            Ok(node)
        }
        Some(tok) => Err(syn::Error::new_spanned(
            tok.clone(),
            "expected node kind, `_`, `@`, or string literal",
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
        // Check for @capture
        if peek_is_at(tokens) {
            let node = parse_query_atom(tokens)?;
            // Check for repetition
            let elem = maybe_wrap_repetition(tokens, quote! {
                yeast::query::QueryListElem::SingleNode(#node)
            })?;
            elems.push(elem);
            continue;
        }

        // Check for parenthesized group
        if peek_is_group(tokens, Delimiter::Parenthesis) {
            let group = expect_group(tokens, Delimiter::Parenthesis)?;
            let mut inner = group.stream().into_iter().peekable();

            // Check for repetition after the group
            if peek_is_repetition(tokens) {
                // This is a repeated subsequence: (patterns)* or (patterns)+ or (patterns)?
                let rep = expect_repetition(tokens)?;
                let sub_elems = parse_query_list(&mut inner)?;
                elems.push(quote! {
                    yeast::query::QueryListElem::Repeated {
                        children: vec![#(#sub_elems),*],
                        rep: #rep,
                    }
                });
            } else {
                // This is a single parenthesized node
                let node = parse_query_node_inner(&mut inner)?;
                // Check for @capture after
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

        // Check for _ (wildcard)
        if peek_is_underscore(tokens) {
            tokens.next();
            let node = quote! { yeast::query::QueryNode::Any() };
            let node = maybe_wrap_capture(tokens, node)?;
            elems.push(quote! {
                yeast::query::QueryListElem::SingleNode(#node)
            });
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
