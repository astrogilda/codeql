#![cfg(test)]

use yeast::dump::dump_ast;
use yeast::*;

/// Helper: parse Ruby source, apply rules, return dump of result.
fn run_and_dump(input: &str, rules: Vec<Rule>) -> String {
    let runner = Runner::new(tree_sitter_ruby::LANGUAGE.into(), rules);
    let ast = runner.run(input);
    dump_ast(&ast, ast.get_root(), input)
}

/// Helper: parse Ruby source with no rules, return dump.
fn parse_and_dump(input: &str) -> String {
    run_and_dump(input, vec![])
}

// ---- Parsing tests ----

#[test]
fn test_parse_assignment() {
    let dump = parse_and_dump("x = 1");
    assert_eq!(dump.trim(), "\
program
  assignment
    left: identifier \"x\"
    right: integer \"1\"");
}

#[test]
fn test_parse_multiple_assignment() {
    let dump = parse_and_dump("x, y = foo()");
    assert_eq!(dump.trim(), "\
program
  assignment
    left:
      left_assignment_list
        identifier \"x\"
        identifier \"y\"
    right:
      call
        arguments:
          argument_list
        method: identifier \"foo\"");
}

#[test]
fn test_parse_for_loop() {
    let dump = parse_and_dump("for x in list do\n  y\nend");
    assert_eq!(dump.trim(), "\
program
  for
    body:
      do
        identifier \"y\"
    pattern: identifier \"x\"
    value:
      in
        identifier \"list\"");
}

// ---- Query tests ----

#[test]
fn test_query_match() {
    let runner = Runner::new(tree_sitter_ruby::LANGUAGE.into(), vec![]);
    let ast = runner.run("x = 1");

    let query = yeast::query!(
        (program
            child: (assignment
                left: (_) @left
                right: (_) @right
            )
        )
    );

    let mut captures = yeast::captures::Captures::new();
    let matched = query.do_match(&ast, ast.get_root(), &mut captures).unwrap();
    assert!(matched);
    assert!(captures.get_var("left").is_ok());
    assert!(captures.get_var("right").is_ok());
}

#[test]
fn test_query_no_match() {
    let runner = Runner::new(tree_sitter_ruby::LANGUAGE.into(), vec![]);
    let ast = runner.run("x = 1");

    let query = yeast::query!(
        (program
            child: (call
                method: (_) @m
            )
        )
    );

    let mut captures = yeast::captures::Captures::new();
    let matched = query.do_match(&ast, ast.get_root(), &mut captures).unwrap();
    assert!(!matched);
}

#[test]
fn test_query_repeated_capture() {
    let runner = Runner::new(tree_sitter_ruby::LANGUAGE.into(), vec![]);
    let ast = runner.run("x, y, z = 1");

    let query = yeast::query!(
        (assignment
            left: (left_assignment_list
                (identifier)* @names
            )
        )
    );

    // Match against the assignment node (first named child of program)
    let mut cursor = AstCursor::new(&ast);
    cursor.goto_first_child();
    let assignment_id = cursor.node().id();

    let mut captures = yeast::captures::Captures::new();
    let matched = query.do_match(&ast, assignment_id, &mut captures).unwrap();
    assert!(matched);
    assert_eq!(captures.get_all("names").len(), 3);
}

// ---- Tree builder tests ----

#[test]
fn test_tree_builder() {
    let runner = Runner::new(tree_sitter_ruby::LANGUAGE.into(), vec![]);
    let mut ast = runner.run("x = 1");
    let input = "x = 1";

    let query = yeast::query!(
        (program
            child: (assignment
                left: (_) @left
                right: (_) @right
            )
        )
    );

    let mut captures = yeast::captures::Captures::new();
    query.do_match(&ast, ast.get_root(), &mut captures).unwrap();

    // Swap left and right
    let fresh = yeast::tree_builder::FreshScope::new();
    let mut ctx = yeast::build::BuildCtx::new(&mut ast, &captures, &fresh);
    let new_id = yeast::tree!(ctx,
        (program
            child: (assignment
                left: @right
                right: @left
            )
        )
    );

    let dump = dump_ast(ctx.ast, new_id, input);
    assert_eq!(dump.trim(), "\
program
  assignment
    left: integer \"1\"
    right: identifier \"x\"");
}

// ---- Rule tests ----

fn ruby_rules() -> Vec<Rule> {
    let assign_rule = yeast::rule!(
        (assignment
            left: (left_assignment_list
                (identifier)* @left
            )
            right: (_) @right
        )
        =>
        (assignment
            left: (identifier $tmp)
            right: {right}
        )
        {..left.iter().enumerate().map(|(i, &lhs)|
            yeast::tree!(
                (assignment
                    left: {lhs}
                    right: (element_reference
                        object: (identifier $tmp)
                        (integer #{i})
                    )
                )
            )
        )}
    );

    let for_rule = yeast::rule!(
        (for
            pattern: (_) @pat
            value: (in (_) @val)
            body: (do (_)* @body)
        )
        =>
        (call
            receiver: {val}
            method: (identifier "each")
            block: (block
                parameters: (block_parameters
                    (identifier $tmp)
                )
                body: (block_body
                    (assignment
                        left: {pat}
                        right: (identifier $tmp)
                    )
                    {..body}
                )
            )
        )
    );

    vec![assign_rule, for_rule]
}

#[test]
fn test_desugar_multiple_assignment() {
    let dump = run_and_dump("x, y = e", ruby_rules());
    assert_eq!(dump.trim(), "\
program
  assignment
    left: identifier \"$tmp-0\"
    right: identifier \"e\"
  assignment
    left: identifier \"x\"
    right:
      element_reference
        object: identifier \"$tmp-0\"
        integer \"0\"
  assignment
    left: identifier \"y\"
    right:
      element_reference
        object: identifier \"$tmp-0\"
        integer \"1\"");
}

#[test]
fn test_desugar_for_loop() {
    let dump = run_and_dump("for x in list do\n  y\nend", ruby_rules());
    assert_eq!(dump.trim(), "\
program
  call
    block:
      block
        body:
          block_body
            assignment
              left: identifier \"x\"
              right: identifier \"$tmp-0\"
            identifier \"y\"
        parameters:
          block_parameters
            identifier \"$tmp-0\"
    method: identifier \"each\"
    receiver: identifier \"list\"");
}

#[test]
fn test_shorthand_rule() {
    let rule = yeast::rule!(
        (assignment
            left: (_) @method
            right: (_) @receiver
        )
        => call
    );

    let dump = run_and_dump("x = 1", vec![rule]);
    assert_eq!(dump.trim(), "\
program
  call
    method: identifier \"x\"
    receiver: integer \"1\"");
}

// ---- Cursor tests ----

#[test]
fn test_cursor_navigation() {
    let runner = Runner::new(tree_sitter_ruby::LANGUAGE.into(), vec![]);
    let ast = runner.run("x = 1");
    let mut cursor = AstCursor::new(&ast);

    // Start at root
    assert_eq!(cursor.node().kind(), "program");

    // Go to first child (assignment)
    assert!(cursor.goto_first_child());
    assert_eq!(cursor.node().kind(), "assignment");

    // No sibling
    assert!(!cursor.goto_next_sibling());

    // Go to first child of assignment
    assert!(cursor.goto_first_child());
    assert!(cursor.node().is_named());

    // Go back up
    assert!(cursor.goto_parent());
    assert_eq!(cursor.node().kind(), "assignment");

    assert!(cursor.goto_parent());
    assert_eq!(cursor.node().kind(), "program");

    // Can't go further up
    assert!(!cursor.goto_parent());
}

#[test]
fn test_desugar_for_with_multiple_assignment() {
    let dump = run_and_dump("for a, b in list do\n  x\nend", ruby_rules());
    assert_eq!(dump.trim(), "\
program
  call
    block:
      block
        body:
          block_body
            assignment
              left: identifier \"$tmp-1\"
              right: identifier \"$tmp-0\"
            assignment
              left: identifier \"a\"
              right:
                element_reference
                  object: identifier \"$tmp-1\"
                  integer \"0\"
            assignment
              left: identifier \"b\"
              right:
                element_reference
                  object: identifier \"$tmp-1\"
                  integer \"1\"
            identifier \"x\"
        parameters:
          block_parameters
            identifier \"$tmp-0\"
    method: identifier \"each\"
    receiver: identifier \"list\"");
}
