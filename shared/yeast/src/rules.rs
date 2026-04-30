use crate::{build::BuildCtx, captures::Captures, *};

pub fn rules() -> Vec<Rule> {
    let assign_query = yeast::query!(
        (assignment
            left: (left_assignment_list
                (identifier)* @left
            )
            right: (_) @right
        )
    );
    let assign_transform = |ast: &mut Ast, match_: Captures| {
        let left_ids = match_.get_all("left");
        let mut assigns = Vec::new();

        // Build individual x = tmp[i] assignments
        let mut ctx = BuildCtx::new(ast, &match_);
        for (i, &lhs) in left_ids.iter().enumerate() {
            let tmp = yeast::tree!(ctx, (identifier $tmp));
            let index = ctx.literal("integer", &i.to_string());
            let assign = yeast::tree!(ctx,
                (assignment
                    left: {lhs}
                    right: (element_reference
                        object: {tmp}
                        {index}
                    )
                )
            );
            assigns.push(assign);
        }

        // Build: tmp = rhs, then all the assigns
        yeast::trees!(ctx,
            (assignment
                left: (identifier $tmp)
                right: @right
            )
            {assigns}
        )
    };

    let assign_rule = Rule::new(assign_query, Box::new(assign_transform));

    let for_query = yeast::query!(
        (for
            pattern: (_) @pat
            value: (in (_) @val)
            body: (do (_)* @body)
        )
    );
    let for_transform = |ast: &mut Ast, match_: Captures| {
        let mut ctx = BuildCtx::new(ast, &match_);
        yeast::trees!(ctx,
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
    };

    let for_rule = Rule::new(for_query, Box::new(for_transform));

    vec![assign_rule, for_rule]
}
