//! Extract text and element bounds from Servo's accessibility updates.
//! Text nodes lack bounds (servo/servo#47164), so each run uses its nearest
//! element's box. Tree depth lets nested boxes reserve their cells first.

use std::sync::Arc;

use accesskit_consumer::{Node, Tree, TreeChangeHandler};
use servo::accesskit::{Node as AccessNode, NodeId, Role, Tree as AccessTree, TreeId, TreeUpdate};

use crate::output::{PxRect, TextBox};

/// Nodes of the host tree that Servo's WebView subtree is grafted into.
const HOST_ROOT: NodeId = NodeId(0);
const HOST_GRAFT: NodeId = NodeId(1);

/// Boxes thinner than this in either direction cannot hold a glyph.
const MIN_BOX_PIXELS: f64 = 1.0;

/// Servo's accessibility tree, and the text boxes read out of it.
pub struct Accessibility {
    tree: Tree,
    /// Shared with every frame built since the last update, so a repaint of
    /// unchanged text copies nothing.
    boxes: Arc<[TextBox]>,
    images: Arc<[PxRect]>,
    /// Rebuild text and image bounds only after a tree update.
    stale: bool,
}

impl Accessibility {
    /// Create the host graft before applying updates from the activated WebView.
    pub fn new(webview_tree: TreeId) -> Self {
        let mut root = AccessNode::new(Role::GenericContainer);
        root.set_children(vec![HOST_GRAFT]);

        // Servo's root transform already converts bounds to viewport device pixels.
        let mut graft = AccessNode::new(Role::GenericContainer);
        graft.set_tree_id(webview_tree);

        let update = TreeUpdate {
            nodes: vec![(HOST_ROOT, root), (HOST_GRAFT, graft)],
            tree: Some(AccessTree {
                root: HOST_ROOT,
                toolkit_name: Some("carboxyl".to_owned()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            tree_id: TreeId::ROOT,
            focus: HOST_ROOT,
        };

        Self {
            tree: Tree::new(update, true),
            boxes: Vec::new().into(),
            images: Vec::new().into(),
            stale: true,
        }
    }

    pub fn update(&mut self, update: TreeUpdate) {
        self.tree.update_and_process_changes(update, &mut Ignore);
        self.stale = true;
    }

    /// The page's text boxes in document order, and its image regions.
    pub fn snapshot(&mut self) -> (Arc<[TextBox]>, Arc<[PxRect]>) {
        if self.stale {
            let mut boxes = Vec::new();
            let mut images = Vec::new();
            collect(self.tree.state().root(), 0, None, &mut boxes, &mut images);
            boxes.retain(|text_box: &TextBox| !text_box.text.trim().is_empty());
            self.boxes = boxes.into();
            self.images = images.into();
            self.stale = false;
        }
        (self.boxes.clone(), self.images.clone())
    }
}

/// Walk the tree in document order, attaching each run of text to the nearest
/// enclosing element that has geometry.
fn collect(
    node: Node,
    depth: u16,
    anchor: Option<usize>,
    out: &mut Vec<TextBox>,
    images: &mut Vec<PxRect>,
) {
    if node.is_hidden() {
        return;
    }

    if node.role() == Role::TextRun {
        // Text nodes have no box of their own, so their content belongs to the
        // enclosing element's box. Sibling runs concatenate in document order.
        if let (Some(index), Some(text)) = (anchor, node.value()) {
            out[index].text.push_str(&text);
        }
        return;
    }

    let anchor = match node.bounding_box() {
        Some(rect)
            if rect.width() >= MIN_BOX_PIXELS
                && rect.height() >= MIN_BOX_PIXELS
                && !is_inline(node.role()) =>
        {
            if matches!(
                node.role(),
                Role::Image | Role::Canvas | Role::Video | Role::SvgRoot
            ) {
                images.push(PxRect::new(
                    rect.x0 as f32,
                    rect.y0 as f32,
                    rect.x1 as f32,
                    rect.y1 as f32,
                ));
            }
            out.push(TextBox {
                rect: PxRect::new(
                    rect.x0 as f32,
                    rect.y0 as f32,
                    rect.x1 as f32,
                    rect.y1 as f32,
                ),
                text: String::new(),
                depth,
            });
            Some(out.len() - 1)
        }
        // `display: contents` and friends generate no box; their text belongs
        // to whichever ancestor does have one.
        _ => anchor,
    };

    for child in node.children() {
        collect(child, depth.saturating_add(1), anchor, out, images);
    }
}

/// Phrasing content that flows inside its parent's text. Laid out as a box of
/// its own it would break the parent's words around it, because the terminal's
/// monospace columns never line up with a proportional font's. Its text joins
/// the parent's in document order instead; the pixels still carry its colors.
fn is_inline(role: Role) -> bool {
    matches!(
        role,
        Role::Link
            | Role::Emphasis
            | Role::Strong
            | Role::Code
            | Role::Mark
            | Role::Abbr
            | Role::Time
            | Role::ContentInsertion
            | Role::ContentDeletion
    )
}

/// Boxes are rebuilt on demand; per-node callbacks need no work.
struct Ignore;

impl TreeChangeHandler for Ignore {
    fn node_added(&mut self, _: &Node) {}
    fn node_updated(&mut self, _: &Node, _: &Node) {}
    fn focus_moved(&mut self, _: Option<&Node>, _: Option<&Node>) {}
    fn node_removed(&mut self, _: &Node) {}
}
