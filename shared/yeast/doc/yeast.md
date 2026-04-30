# YEAST — Yet another Elaborator for Abstract Syntax Trees

YEAST is a framework for transforming tree-sitter parse trees before they are
extracted into a CodeQL database. It sits between the tree-sitter parser and
the TRAP extractor, rewriting parts of the AST according to declarative rules.

## Motivation

Tree-sitter grammars describe the **concrete syntax** of a language — every
keyword, operator, and punctuation token appears in the parse tree. CodeQL
analyses often prefer a **simplified abstract syntax** where syntactic sugar
has been removed. YEAST bridges this gap by desugaring the tree-sitter output
into a cleaner form before extraction.

For example, Ruby's `for x in list do ... end` is syntactic sugar for
`list.each { |x| ... }`. A YEAST rule can rewrite the former into the latter
so that CodeQL queries only need to reason about the `.each` form.

## Architecture

```
Source code
    │
    ▼
┌──────────────┐
│  tree-sitter │  Parse source into a concrete syntax tree
│    parser    │
└──────┬───────┘
       │ tree_sitter::Tree
       ▼
┌──────────────┐
│    YEAST     │  Apply desugaring rules, producing a new AST
│   Runner     │
└──────┬───────┘
       │ yeast::Ast
       ▼
┌──────────────┐
│    TRAP      │  Walk the (possibly rewritten) AST and emit TRAP tuples
│  extractor   │
└──────────────┘
```

The entry point is `extract_and_desugar()` in the shared tree-sitter
extractor, which passes a set of rules to the YEAST `Runner`. The original
`extract()` function passes empty rules, leaving the tree unchanged.

## How desugaring works

A YEAST `Rule` has two parts:

1. A **query** that matches nodes in the AST using a tree-sitter-inspired
   pattern language.
2. A **transform** that produces replacement nodes from the match captures.

The `Runner` applies rules by walking the tree bottom-up. At each node, it
tries each rule in order. If a rule's query matches, the node is replaced by
the transform's output, and the rules are re-applied to the result. If no
rule matches, the node is kept and its children are processed recursively.

A rule can replace one node with zero nodes (deletion), one node (rewriting),
or multiple nodes (expansion).

## Query language

Queries use a syntax inspired by
[tree-sitter queries](https://tree-sitter.github.io/tree-sitter/using-parsers/queries/index.html),
written inside the `yeast::query!()` proc macro.

### Node patterns

```rust
// Match any named node
(_)

// Match a node of a specific kind
(assignment)

// Match an unnamed token by its text
("end")
```

### Fields

```rust
// Match a node with specific fields
(assignment
    left: (identifier) @lhs
    right: (_) @rhs
)
```

Fields are matched by name. Unmentioned fields are ignored — the pattern
`(assignment left: (_) @x)` matches any `assignment` node regardless of
what's in `right`.

### Captures

Captures bind matched nodes to names for use in the transform. A capture
`@name` always follows the pattern it captures:

```rust
(identifier) @name          // capture an identifier node
(_) @value                  // capture any named node
(identifier)* @items        // capture each repeated match
```

### Unnamed children

Patterns that appear after all named fields match unnamed (positional)
children. Named node patterns like `(_)` automatically skip unnamed tokens
(keywords, operators, punctuation), matching tree-sitter semantics:

```rust
(for
    pattern: (_) @pat             // named field
    value: (in (_) @val)          // "in" token is skipped automatically
    body: (do (_)* @body)         // "do" and "end" tokens skipped
)
```

### Repetitions

```rust
(_)*                   // zero or more
(_)+                   // one or more
(_)?                   // zero or one
(identifier)* @names   // capture each repeated match
```

## Builder language

Builders construct new AST nodes from captures. They use a similar syntax
inside `yeast::tree_builder!()` and `yeast::trees_builder!()`.

### Capture references

In builders, `@name` references a captured value from the query match:

```rust
yeast::tree_builder!(
    (assignment
        left: @rhs       // insert the captured @rhs node as the left child
        right: @lhs       // insert the captured @lhs node as the right child
    )
)
```

### Literal nodes

Create a leaf node with a fixed text content:

```rust
(identifier "each")          // an identifier node whose text is "each"
```

### Fresh identifiers

Create a leaf node with an auto-generated unique name. All occurrences of the
same `$name` within one rule application share the same generated value:

```rust
(block
    parameters: (block_parameters
        (identifier $tmp)         // generates e.g. "$tmp-0"
    )
    body: (block_body
        (assignment
            left: @pat
            right: (identifier $tmp)   // same "$tmp-0" value
        )
    )
)
```

Use `build_tree_with_fresh()` / `build_trees_with_fresh()` when you need the
same fresh scope across multiple `build_tree` calls.

### Repeated splicing

In `trees_builder!()`, `(@captures)*` splices a repeated capture into the
output, expanding once per captured value:

```rust
yeast::trees_builder!(
    (assignment left: @tmp right: @right)
    (@assigns)*                           // one node per captured assign
)
```

## Complete example: for-loop desugaring

This rule rewrites Ruby's `for pat in val do body end` into
`val.each { |tmp| pat = tmp; body }`:

```rust
// Query: match for-loops
let query = yeast::query!(
    (for
        pattern: (_) @pat
        value: (in (_) @val)
        body: (do (_)* @body)
    )
);

// Transform: build the .each block form
let transform = |ast: &mut Ast, match_: Captures| {
    yeast::trees_builder!(
        (call
            receiver: @val
            method: (identifier "each")
            block: (block
                parameters: (block_parameters
                    (identifier $tmp)
                )
                body: (block_body
                    (assignment
                        left: @pat
                        right: (identifier $tmp)
                    )
                    (@body)*
                )
            )
        )
    )
    .build_trees(ast, &match_)
    .unwrap()
};

let rule = Rule::new(query, Box::new(transform));
```

## Integration with the extractor

YEAST integrates with the shared tree-sitter extractor via two mechanisms:

1. **`extract_and_desugar()`** — like `extract()`, but takes a
   `Vec<yeast::Rule>` to apply before TRAP extraction.

2. **`LanguageSpec::output_node_types`** — when desugaring produces an AST
   with different node types than the tree-sitter grammar, this field points
   to a separate `node-types.json` describing the output schema.

Languages that don't use desugaring simply call `extract()`, which passes
empty rules internally.
