use crate::{captures::Captures, tree_builder::FreshScope, *};

pub fn rules() -> Vec<Rule> {
    let assign_query = yeast::query!(
        (assignment
            left: (left_assignment_list
                ((identifier) @left (",")?)*
            )
            right: @right
        )
    );
    let assign_transform = |ast: &mut Ast, mut match_: Captures| {
        let fresh = FreshScope::new();

        let tmp_lhs = yeast::tree_builder!((identifier $tmp))
            .build_tree_with_fresh(ast, &match_, &fresh).unwrap();
        match_.insert("tmp_lhs", tmp_lhs);

        let mut i = 0;
        match_.map_captures_to("left", "assigns", &mut |old_id| {
            let mut local_capture = Captures::new();
            local_capture.insert("lhs", old_id);
            local_capture.insert(
                "tmp",
                yeast::tree_builder!((identifier $tmp))
                    .build_tree_with_fresh(ast, &local_capture, &fresh).unwrap(),
            );
            let index: i32 = i;
            i += 1;
            local_capture.insert(
                "index",
                ast.create_named_token("integer", index.to_string()),
            );
            yeast::tree_builder!(
                (assignment
                    left: @lhs
                    right: (element_reference
                        object: @tmp
                        @index
                    )
                )
            )
            .build_tree_with_fresh(ast, &local_capture, &fresh)
            .unwrap()
        });

        yeast::trees_builder!(
            (assignment
                left: @tmp_lhs
                right: @right
            )
            (@assigns)*
        )
        .build_trees_with_fresh(ast, &match_, &fresh)
        .unwrap()
    };

    let assign_rule = Rule::new(assign_query, Box::new(assign_transform));

    let for_query = yeast::query!(
        (for
            pattern: @pat
            value: (in (_) @val)
            body: (do "do"? (@body)*)
        )
    );
    let for_transform = |ast: &mut Ast, match_: Captures| {
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

    let for_rule = Rule::new(for_query, Box::new(for_transform));

    let end_query = yeast::query!(("end"));
    let end_transform = |_ast: &mut Ast, _match: Captures| vec![];
    let end_rule = Rule::new(end_query, Box::new(end_transform));
    vec![assign_rule, for_rule, end_rule]
}
