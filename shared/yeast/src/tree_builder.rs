use crate::{captures::Captures, Ast, Id};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub enum TreeBuilder {
    Node {
        kind: &'static str,
        children: Vec<(&'static str, Vec<TreeChildBuilder>)>,
    },
    Capture {
        capture: &'static str,
    },
}

#[derive(Debug, Clone)]
pub enum TreeChildBuilder {
    Repeated {
        child: TreeBuilder,
    },
    SingleNode(TreeBuilder),
}

impl TreeChildBuilder {
    fn get_opt_contained(&self) -> BTreeSet<&'static str> {
        match self {
            TreeChildBuilder::Repeated { child } => child.get_opt_contained(),
            TreeChildBuilder::SingleNode(node) => node.get_opt_contained(),
        }
    }

    fn build_tree(
        &self,
        target: &mut Ast,
        vars: &Captures,
        child_ids: &mut Vec<Id>,
    ) -> Result<(), String> {
        match self {
            TreeChildBuilder::Repeated { child } => {
                let repeated_ids = self.get_opt_contained();

                for sub_captures in vars.un_star(&repeated_ids)? {
                    child_ids.push(child.build_tree(target, &sub_captures)?)
                }
                Ok(())
            }
            TreeChildBuilder::SingleNode(node) => {
                child_ids.push(node.build_tree(target, vars)?);
                Ok(())
            }
        }
    }
}

impl TreeBuilder {
    fn get_opt_contained(&self) -> BTreeSet<&'static str> {
        match self {
            TreeBuilder::Node { kind: _, children } => {
                let mut contained = BTreeSet::new();
                for (_, children) in children {
                    for child in children {
                        contained.extend(child.get_opt_contained());
                    }
                }
                contained
            }
            TreeBuilder::Capture { capture } => {
                let mut contained = BTreeSet::new();
                contained.insert(*capture);
                contained
            }
        }
    }

    pub fn build_tree(&self, target: &mut Ast, vars: &Captures) -> Result<Id, String> {
        match self {
            TreeBuilder::Capture { capture } => vars.get_var(capture),
            TreeBuilder::Node { kind, children } => {
                let ast_kind = target.id_for_node_kind(kind).ok_or_else(||
                    format!("Node kind {} does not exist in language", kind)
                )?;

                let child_vars = children.iter().map(|(field, children)| {
                    let mut child_ids = Vec::new();
                    for child in children {
                        child.build_tree(target, vars, &mut child_ids)?;
                    }
                    let field_id = target
                        .field_id_for_name(field)
                        .ok_or(format!("Field {} does not exist in language", field))?;
                    Ok((field_id, child_ids))
                }).collect::<Result<_,String>>()?;
                Ok(target.create_node(ast_kind, "".into(), child_vars, true))
            }
        }
    }
}

pub struct TreesBuilder {
    pub children: Vec<TreeChildBuilder>,
}

impl TreesBuilder {
    pub fn build_trees(&self, target: &mut Ast, vars: &Captures) -> Result<Vec<Id>, String> {
        let mut child_ids = Vec::new();
        for child in &self.children {
            child.build_tree(target, vars, &mut child_ids)?;
        }
        Ok(child_ids)
    }
}
