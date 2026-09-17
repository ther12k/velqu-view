//! The Velqu DOM: a flat, arena-based tree produced by one document parse.
//!
//! Identity rules (ADR 0005): a `NodeId` identifies a node *within one
//! parse*. The DOM never holds style, layout, or paint state. Fixture-facing
//! identity is the author-written `data-vv-test` attribute, extracted here so
//! later stages (layout facts) never depend on `NodeId` stability.

/// Index into [`Dom::nodes`]; valid only for the `Dom` that issued it.
pub(crate) type NodeId = usize;

/// One element attribute (`name="value"`), order-preserved from the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attribute {
    pub name: String,
    pub value: String,
}

/// The payload of one DOM node.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NodeData {
    /// The implicit document root.
    Document,
    /// `<!doctype …>` — retained as a marker, never rendered.
    Doctype,
    /// An element with its (lowercased) tag name and attributes.
    Element {
        name: String,
        attrs: Vec<Attribute>,
        /// `data-vv-test` value if present; the fixture-facing identity.
        fixture_id: Option<String>,
    },
    /// Character data. Adjacent text is coalesced at insertion time, so a
    /// text node always has non-text neighbors.
    Text(String),
    /// Comments are parsed; their text is retained for diagnostics.
    Comment(String),
    /// Processing-instruction fallback: HTML has no PIs, payload is dropped.
    Pi,
}

/// One tree node: payload plus tree links.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub data: NodeData,
}

/// A parsed document tree.
#[derive(Debug, Clone)]
pub(crate) struct Dom {
    nodes: Vec<Node>,
    document: NodeId,
}

impl Dom {
    /// An empty document tree containing only the root.
    pub(crate) fn empty() -> Self {
        let mut nodes = Vec::new();
        let document = nodes.len();
        nodes.push(Node {
            parent: None,
            children: Vec::new(),
            data: NodeData::Document,
        });
        Self { nodes, document }
    }

    /// The document root's id.
    pub(crate) fn document(&self) -> NodeId {
        self.document
    }

    /// Borrows a node by id.
    pub(crate) fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    /// Mutably borrows a node by id.
    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    /// Number of allocated nodes, used to validate document-scoped handles.
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Iterates every allocated node with its id (allocation order).
    // Query surface consumed by cascade/layout in the following M2a commits;
    // until each lands, only tests reference some of these.
    #[allow(dead_code)]
    pub(crate) fn nodes_iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter().enumerate()
    }

    /// The element's tag name, or `None` for non-elements.
    #[allow(dead_code)]
    pub(crate) fn tag_name(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id].data {
            NodeData::Element { name, .. } => Some(name),
            _ => None,
        }
    }

    /// The node's `data-vv-test` fixture id, or `None`.
    #[allow(dead_code)]
    pub(crate) fn fixture_id(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id].data {
            NodeData::Element { fixture_id, .. } => fixture_id.as_deref(),
            _ => None,
        }
    }

    /// The element's first attribute with `name`, or `None`.
    pub(crate) fn attribute(&self, id: NodeId, name: &str) -> Option<&str> {
        match &self.nodes[id].data {
            NodeData::Element { attrs, .. } => attrs
                .iter()
                .find(|a| a.name == name)
                .map(|a| a.value.as_str()),
            _ => None,
        }
    }

    /// Depth-first pre-order visit of every node (document root included).
    #[allow(dead_code)]
    pub(crate) fn walk(&self, mut visit: impl FnMut(NodeId, &Node)) {
        let mut stack = vec![self.document];
        while let Some(id) = stack.pop() {
            visit(id, &self.nodes[id]);
            for child in self.nodes[id].children.iter().rev() {
                stack.push(*child);
            }
        }
    }

    /// All `data-vv-test` values in document order (duplicate values are
    /// reported once, in first-seen order).
    #[allow(dead_code)]
    pub(crate) fn fixture_ids(&self) -> Vec<String> {
        let mut seen = Vec::new();
        self.walk(|_id, node| {
            if let NodeData::Element {
                fixture_id: Some(key),
                ..
            } = &node.data
            {
                if !seen.iter().any(|s: &String| s == key) {
                    seen.push(key.clone());
                }
            }
        });
        seen
    }

    /// Concatenated descendant text (no whitespace normalization; that is a
    /// layout-stage concern).
    #[allow(dead_code)]
    pub(crate) fn descendant_text(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.walk_from(id, |_id, node| {
            if let NodeData::Text(text) = &node.data {
                out.push_str(text);
            }
        });
        out
    }

    /// Depth-first pre-order visit of `id`'s subtree (the node itself first).
    #[allow(dead_code)]
    pub(crate) fn walk_from(&self, start: NodeId, mut visit: impl FnMut(NodeId, &Node)) {
        let mut stack = vec![start];
        while let Some(id) = stack.pop() {
            visit(id, &self.nodes[id]);
            for child in self.nodes[id].children.iter().rev() {
                stack.push(*child);
            }
        }
    }

    // -- construction (used by the html5ever sink) ------------------------

    /// Allocates a detached node (no parent, no children).
    pub(crate) fn create_detached(&mut self, data: NodeData) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent: None,
            children: Vec::new(),
            data,
        });
        id
    }

    pub(crate) fn create_element(&mut self, name: String, attrs: Vec<Attribute>) -> NodeId {
        let fixture_id = attrs
            .iter()
            .find(|attr| attr.name == "data-vv-test")
            .map(|attr| attr.value.clone());
        self.create_detached(NodeData::Element {
            name,
            attrs,
            fixture_id,
        })
    }

    /// Inserts `child` directly after `sibling` under `sibling`'s parent.
    pub(crate) fn insert_after(&mut self, sibling: NodeId, child: NodeId) {
        let parent = self.nodes[sibling].parent;
        if let Some(parent) = parent {
            let position = self.nodes[parent]
                .children
                .iter()
                .position(|c| *c == sibling)
                .expect("sibling is a child of its parent");
            self.nodes[child].parent = Some(parent);
            self.nodes[parent].children.insert(position + 1, child);
        }
    }

    /// Inserts a text node as the last child of `parent`, coalescing with a
    /// preceding text sibling.
    pub(crate) fn append_text(&mut self, parent: NodeId, text: String) -> NodeId {
        if let Some(&last) = self.nodes[parent].children.last() {
            if let NodeData::Text(existing) = &mut self.nodes[last].data {
                existing.push_str(&text);
                return last;
            }
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            parent: Some(parent),
            children: Vec::new(),
            data: NodeData::Text(text),
        });
        self.nodes[parent].children.push(id);
        id
    }

    /// Inserts `child` as the last child of `parent` (no coalescing).
    pub(crate) fn append(&mut self, parent: NodeId, child: NodeId) {
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
    }

    /// Inserts `child` immediately before `sibling` under `sibling`'s parent.
    pub(crate) fn insert_before(&mut self, sibling: NodeId, child: NodeId) {
        let parent = self.nodes[sibling].parent;
        if let Some(parent) = parent {
            let position = self.nodes[parent]
                .children
                .iter()
                .position(|c| *c == sibling)
                .expect("sibling is a child of its parent");
            self.nodes[child].parent = Some(parent);
            self.nodes[parent].children.insert(position, child);
        }
    }

    /// Detaches `child` from its parent (no-op when already detached).
    pub(crate) fn detach(&mut self, child: NodeId) {
        if let Some(parent) = self.nodes[child].parent.take() {
            self.nodes[parent].children.retain(|c| *c != child);
        }
    }

    /// Moves all children of `from` under `to`, preserving order.
    pub(crate) fn reparent_children(&mut self, from: NodeId, to: NodeId) {
        let children = std::mem::take(&mut self.nodes[from].children);
        for child in &children {
            self.nodes[*child].parent = Some(to);
        }
        self.nodes[to].children.extend(children);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Dom {
        let mut dom = Dom::empty();
        let html = dom.create_element(
            "html".into(),
            vec![Attribute {
                name: "lang".into(),
                value: "en".into(),
            }],
        );
        dom.append(dom.document(), html);
        let body = dom.create_element("body".into(), Vec::new());
        dom.append(html, body);
        let card = dom.create_element(
            "div".into(),
            vec![Attribute {
                name: "data-vv-test".into(),
                value: "card".into(),
            }],
        );
        dom.append(body, card);
        let p = dom.create_element("p".into(), Vec::new());
        dom.append(card, p);
        dom.append_text(p, "hello ".into());
        dom.append_text(p, "world".into());
        dom
    }

    #[test]
    fn tree_links_are_consistent() {
        let dom = sample();
        let body = dom
            .nodes_iter()
            .find(|(id, _)| dom.tag_name(*id) == Some("body"))
            .map(|(id, _)| id)
            .unwrap();
        assert_eq!(dom.node(body).children.len(), 1);
        assert_eq!(
            dom.node(dom.node(body).children[0]).parent,
            Some(body),
            "child->parent link matches parent->children"
        );
    }

    #[test]
    fn text_coalescing_merges_adjacent_runs() {
        let dom = sample();
        let text = dom.descendant_text(dom.document());
        assert_eq!(text, "hello world");
    }

    #[test]
    fn fixture_ids_are_extracted_in_order() {
        let dom = sample();
        assert_eq!(dom.fixture_ids(), ["card"]);
        assert_eq!(dom.fixture_id(3), Some("card"));
    }

    #[test]
    fn duplicate_fixture_ids_are_reported_once() {
        let mut dom = Dom::empty();
        let a = dom.create_element(
            "div".into(),
            vec![Attribute {
                name: "data-vv-test".into(),
                value: "x".into(),
            }],
        );
        dom.append(dom.document(), a);
        let b = dom.create_element(
            "div".into(),
            vec![Attribute {
                name: "data-vv-test".into(),
                value: "x".into(),
            }],
        );
        dom.append(dom.document(), b);
        assert_eq!(dom.fixture_ids(), ["x"]);
    }
}
