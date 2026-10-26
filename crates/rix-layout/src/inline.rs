
use crate::tree::{AnonymousBoxType, LayoutBoxId, LayoutTree};
use rix_css::computed::{ComputedStyle, Display};
use rix_dom::node::NodeData;
use rix_dom::tree::Document;
use rix_dom::NodeId;
use std::collections::HashMap;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineFlowItem {
    TextRun(usize),
    InlineStart(LayoutBoxId),
    InlineEnd(LayoutBoxId),
    Atomic(LayoutBoxId),
    ForcedBreak(LayoutBoxId),
    BlockBoundary(LayoutBoxId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct InlineTextRun {
    pub source_node: NodeId,
    pub source_box: Option<LayoutBoxId>,
    pub style: ComputedStyle,
    pub range: Range<usize>,
    pub forced_break: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InlineFormattingContext {
    pub block: LayoutBoxId,
    pub text: String,
    pub runs: Vec<InlineTextRun>,
    pub items: Vec<InlineFlowItem>,
    pub(crate) pending_atomic_space: bool,
}

fn establishes_inline_context(display: Display) -> bool {
    matches!(
        display,
        Display::Block | Display::InlineBlock | Display::ListItem | Display::TableCell
    )
}

pub(crate) fn collect_inline_contexts(
    doc: &Document,
    styles: &HashMap<NodeId, ComputedStyle>,
    tree: &LayoutTree,
) -> (Vec<InlineFormattingContext>, HashMap<LayoutBoxId, usize>) {
    let mut contexts = Vec::new();
    let mut context_map = HashMap::new();
    for block in tree.iter().filter(|layout_box| {
        establishes_inline_context(layout_box.style.display)
            && matches!(layout_box.box_type, AnonymousBoxType::None)
    }) {
        let Some(node) = block.dom_node else {
            continue;
        };
        if block.children.iter().any(|child_id| {
            tree.get(*child_id).is_some_and(|child| {
                child.dom_node == Some(node) && matches!(child.box_type, AnonymousBoxType::Text(_))
            })
        }) {
            continue;
        }
        let mut context = InlineFormattingContext {
            block: block.id,
            text: String::new(),
            runs: Vec::new(),
            items: Vec::new(),
            pending_atomic_space: false,
        };
        doc.for_each_composed_child(node, |child| {
            collect_node(
                doc,
                tree,
                child,
                &block.style,
                styles,
                &mut context,
            );
        });

        if context.items.iter().any(|item| {
            matches!(
                item,
                InlineFlowItem::TextRun(_)
                    | InlineFlowItem::InlineStart(_)
                    | InlineFlowItem::Atomic(_)
                    | InlineFlowItem::ForcedBreak(_)
            )
        }) {
            context_map.insert(context.block, contexts.len());
            contexts.push(context);
        }
    }
    (contexts, context_map)
}

fn collect_node(
    doc: &Document,
    tree: &LayoutTree,
    node: NodeId,
    inherited: &ComputedStyle,
    styles: &HashMap<NodeId, ComputedStyle>,
    context: &mut InlineFormattingContext,
) {
    match doc.get(node) {
        Some(NodeData::Text(text)) => {
            let style = styles.get(&node).unwrap_or(inherited);
            let source_box = tree.box_for_node(node).map(|b| b.id);
            append_text_run(context, node, source_box, style, text);
        }
        Some(NodeData::Element(element)) => {
            let style = styles.get(&node).unwrap_or(inherited);
            if style.display == Display::None {
                return;
            }
            let Some(layout_box) = tree.box_for_node(node).map(|b| b.id) else {
                return;
            };

            if style.display.is_inline_level() {
                if element.name.local.as_str() == "br" {
                    let start = context.text.len();
                    context.text.push('\n');
                    context.runs.push(InlineTextRun {
                        source_node: node,
                        source_box: Some(layout_box),
                        style: text_style(style),
                        range: start..context.text.len(),
                        forced_break: true,
                    });
                    context.items.push(InlineFlowItem::ForcedBreak(layout_box));
                    context.pending_atomic_space = false;
                } else if style.display.is_atomic_inline()
                    || tree
                        .get(layout_box)
                        .is_some_and(|box_data| box_data.is_replaced)
                {
                    context.items.push(InlineFlowItem::Atomic(layout_box));
                    context.pending_atomic_space = true;
                } else {
                    context.items.push(InlineFlowItem::InlineStart(layout_box));
                    doc.for_each_composed_child(node, |child| {
                        collect_node(doc, tree, child, style, styles, context);
                    });
                    context.items.push(InlineFlowItem::InlineEnd(layout_box));
                }
            } else {
                context
                    .items
                    .push(InlineFlowItem::BlockBoundary(layout_box));
                context.pending_atomic_space = false;
            }
        }
        _ => {}
    }
}

fn append_text_run(
    context: &mut InlineFormattingContext,
    source_node: NodeId,
    source_box: Option<LayoutBoxId>,
    style: &ComputedStyle,
    raw_text: &str,
) {
    let run_index = context.runs.len();
    let start = context.text.len();
    context.runs.push(InlineTextRun {
        source_node,
        source_box,
        style: text_style(style),
        range: start..start,
        forced_break: false,
    });
    context.items.push(InlineFlowItem::TextRun(run_index));

    let white_space = style.white_space.as_str();
    for ch in raw_text.chars() {
        if matches!(white_space, "pre" | "pre-wrap" | "break-spaces") {
            append_char(context, run_index, ch);
            context.pending_atomic_space = false;
        } else if white_space == "pre-line" && matches!(ch, '\n' | '\r') {
            if !context.text.ends_with('\n') {
                append_char(context, run_index, '\n');
            }
            context.pending_atomic_space = false;
        } else if is_collapsible_space(ch) {
            if context.pending_atomic_space {
                append_char(context, run_index, ' ');
                context.pending_atomic_space = false;
            } else if !context.text.is_empty()
                && !context.text.ends_with(' ')
                && !context.text.ends_with('\n')
            {
                append_char(context, run_index, ' ');
            }
        } else {
            append_char(context, run_index, ch);
            context.pending_atomic_space = false;
        }
    }

    if context.runs[run_index].range.is_empty() {
        context.runs.pop();
        context.items.pop();
    }
}

fn append_char(context: &mut InlineFormattingContext, run_index: usize, ch: char) {
    context.text.push(ch);
    if let Some(run) = context.runs.get_mut(run_index) {
        run.range.end = context.text.len();
    }
}

fn is_collapsible_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

fn text_style(style: &ComputedStyle) -> ComputedStyle {
    let mut text_style = style.clone();
    text_style.display = Display::Inline;
    text_style
}
