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

## Template language

Templates construct new AST nodes using the `tree!` and `trees!` macros.
Both take a `BuildCtx` as their first argument, which holds the AST,
captures from the query match, and a fresh identifier scope.

```rust
let mut ctx = BuildCtx::new(ast, &captures);
```

### `tree!` — build a single node

`tree!(ctx, ...)` returns a single node `Id`:

```rust
let id = yeast::tree!(ctx,
    (assignment
        left: @lhs
        right: @rhs
    )
);
```

### `trees!` — build multiple nodes

`trees!(ctx, ...)` returns `Vec<Id>`:

```rust
let ids = yeast::trees!(ctx,
    (assignment left: @tmp right: @right)
    (@body)*
);
```

### Capture references

`@name` references a captured value from the query match:

```rust
(assignment
    left: @rhs       // insert the captured @rhs node as the left child
    right: @lhs       // insert the captured @lhs node as the right child
)
```

### Literal nodes

`(kind "text")` creates a leaf node with fixed text content:

```rust
(identifier "each")          // an identifier node whose text is "each"
```

### Computed literals

`(kind #{expr})` creates a leaf node whose content is `expr.to_string()`:

```rust
(integer #{i})               // an integer node with the value of i
(identifier #{name})         // an identifier from a Rust variable
```

### Fresh identifiers

`(kind $name)` creates a leaf node with an auto-generated unique name. All
occurrences of the same `$name` within one `BuildCtx` share the same value:

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

### Embedded Rust expressions

`{expr}` embeds a Rust expression that returns a single node `Id`:

```rust
(assignment
    left: {some_node_id}       // insert a pre-built node
    right: @rhs
)
```

`{..expr}` splices a `Vec<Id>` (or any iterable of `Id`):

```rust
yeast::trees!(ctx,
    (assignment left: @tmp right: @right)
    {..extra_nodes}                        // splice a Vec<Id>
)
```

### Repeated capture splicing

`(@name)*` splices a repeated capture, inserting one node per captured value:

```rust
yeast::trees!(ctx,
    (first_node ...)
    (@body)*              // one node per captured @body value
)
```

## Complete example: for-loop desugaring

This rule rewrites Ruby's `for pat in val do body end` into
`val.each { |tmp| pat = tmp; body }`:

```rust
let query = yeast::query!(
    (for
        pattern: (_) @pat
        value: (in (_) @val)
        body: (do (_)* @body)
    )
);

let transform = |ast: &mut Ast, match_: Captures| {
    let mut ctx = BuildCtx::new(ast, &match_);
    vec![yeast::tree!(ctx,
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
    )]
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
