use crate::{build::BuildCtx, captures::Captures, *};

pub fn rules() -> Vec<Rule> {
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
        {..left.iter().enumerate().map(|(i, &lhs)| {
            yeast::tree!(
                (assignment
                    left: {lhs}
                    right: (element_reference
                        object: (identifier $tmp)
                        (integer #{i})
                    )
                )
            )
        })}
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
