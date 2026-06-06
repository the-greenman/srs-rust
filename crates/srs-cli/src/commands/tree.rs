use crate::commands::{with_store, CliContext, TreeArgs};
use crate::output;
use crate::payload::{TreeNodePayload, TreePayload};
use anyhow::Result;
use srs_repository::tree_service::{self, TreeNode, TreeOptions};

pub fn dispatch(ctx: CliContext, args: TreeArgs) -> Result<String> {
    let root_ids = if args.from.is_empty() {
        None
    } else {
        Some(args.from)
    };
    let options = TreeOptions {
        root_ids,
        container_id: ctx.container_id.clone(),
        relation_type: args.relation_type,
        max_depth: args.depth,
        type_filter: args.type_filter,
    };
    match with_store(&ctx, |store| Ok(tree_service::build_tree(store, options)?)) {
        Ok(result) => {
            let roots: Vec<TreeNodePayload> = result.roots.iter().map(map_node).collect();
            let text = render_ascii_tree(&result.roots);
            output::serialize(
                "tree",
                TreePayload {
                    roots,
                    text,
                    diagnostics: result.diagnostics,
                },
            )
        }
        Err(e) => Ok(output::err("tree", vec![e.to_string()])),
    }
}

fn map_node(node: &TreeNode) -> TreeNodePayload {
    TreeNodePayload {
        instance_id: node.instance_id.clone(),
        label: node.label.clone(),
        type_namespace: node.type_namespace.clone(),
        type_name: node.type_name.clone(),
        lifecycle_state: node.lifecycle_state.clone(),
        depth: node.depth,
        children: node.children.iter().map(map_node).collect(),
        cycle_pruned: node.cycle_pruned,
    }
}

fn render_ascii_tree(nodes: &[TreeNode]) -> String {
    let mut out = String::new();
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == nodes.len() - 1;
        render_node(node, "", is_last, &mut out);
    }
    out
}

fn render_node(node: &TreeNode, prefix: &str, is_last: bool, out: &mut String) {
    let connector = if is_last { "└── " } else { "├── " };
    let type_tag = format!("[{}/{}]", node.type_namespace, node.type_name);
    let id_short = &node.instance_id[..8.min(node.instance_id.len())];
    let state_tag = node
        .lifecycle_state
        .as_deref()
        .map(|s| format!("  [{s}]"))
        .unwrap_or_default();
    let cycle_tag = if node.cycle_pruned { "  ↻ cycle" } else { "" };
    out.push_str(&format!(
        "{prefix}{connector}{label} {type_tag} ({id_short}){state_tag}{cycle_tag}\n",
        label = node.label,
    ));

    let child_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });
    for (i, child) in node.children.iter().enumerate() {
        let child_is_last = i == node.children.len() - 1;
        render_node(child, &child_prefix, child_is_last, out);
    }
}
