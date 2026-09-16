//! HTML parsing: lowers source text into the Velqu DOM via html5ever.
//!
//! html5ever is used strictly as a spec-correct *tokenizer/tree builder*;
//! the output is lowered immediately into VelquView's small DOM model
//! (`dom::Dom`), never into a browser-sized tree type. Parsing is lenient —
//! malformed input produces the html5ever-corrected tree, and parse errors
//! are counted for later diagnostics rather than failing the load.

use std::cell::RefCell;
use std::collections::HashMap;

use html5ever::tendril::StrTendril;
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, LocalName, Namespace, ParseOpts, QualName};

use crate::dom::{Attribute as DomAttribute, Dom, Node, NodeData, NodeId};

/// An element name owned by the sink, returned from `elem_name` by value.
///
/// The `TreeSink::elem_name` signature forces a borrow of the sink, but the
/// DOM sits behind a `RefCell`; returning an owned name (atoms are cheap)
/// sidesteps the borrow without exposing the cell.
#[derive(Debug, Clone)]
struct OwnedElemName {
    ns: Namespace,
    local: LocalName,
}

impl html5ever::tree_builder::ElemName for OwnedElemName {
    fn ns(&self) -> &Namespace {
        &self.ns
    }

    fn local_name(&self) -> &LocalName {
        &self.local
    }
}

/// Counts html5ever parse errors (surfaces later as diagnostics).
#[derive(Debug)]
pub(crate) struct ParseReport {
    pub parse_errors: u32,
    pub quirks_mode: QuirksMode,
}

struct DomSink {
    dom: RefCell<Dom>,
    /// QualName per element node (sink-side; the DOM keeps plain strings).
    names: RefCell<HashMap<NodeId, QualName>>,
    report: RefCell<ParseReport>,
}

impl DomSink {
    fn to_dom_attribute(attr: &Attribute) -> DomAttribute {
        DomAttribute {
            name: attr.name.local.to_string(),
            value: attr.value.to_string(),
        }
    }

    /// Converts a sink text payload into a detached DOM node when needed.
    fn detach_text(&self, text: StrTendril) -> NodeId {
        self.dom
            .borrow_mut()
            .create_detached(NodeData::Text(text.to_string()))
    }
}

impl TreeSink for DomSink {
    type Handle = NodeId;
    type Output = Dom;
    type ElemName<'a>
        = OwnedElemName
    where
        Self: 'a;

    fn finish(self) -> Dom {
        self.dom.into_inner()
    }

    fn parse_error(&self, _msg: std::borrow::Cow<'static, str>) {
        self.report.borrow_mut().parse_errors += 1;
    }

    fn get_document(&self) -> NodeId {
        self.dom.borrow().document()
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> OwnedElemName {
        // The tree builder only calls this on elements, which always
        // registered their QualName in create_element.
        let name = self
            .names
            .borrow()
            .get(target)
            .cloned()
            .expect("element has a QualName");
        OwnedElemName {
            ns: name.ns,
            local: name.local,
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> NodeId {
        let dom_attrs: Vec<DomAttribute> = attrs.iter().map(Self::to_dom_attribute).collect();
        let id = self
            .dom
            .borrow_mut()
            .create_element(name.local.to_string(), dom_attrs);
        self.names.borrow_mut().insert(id, name);
        id
    }

    fn create_comment(&self, text: StrTendril) -> NodeId {
        self.dom
            .borrow_mut()
            .create_detached(NodeData::Comment(text.to_string()))
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> NodeId {
        // Processing instructions do not exist in HTML; keep the node
        // shape stable but drop the payload.
        self.dom.borrow_mut().create_detached(NodeData::Pi)
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(id) => self.dom.borrow_mut().append(*parent, id),
            NodeOrText::AppendText(text) => {
                self.dom.borrow_mut().append_text(*parent, text.to_string());
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let has_parent = self.dom.borrow().node(*prev_element).parent.is_some();
        if !has_parent {
            self.append(element, child);
            return;
        }
        match child {
            NodeOrText::AppendNode(id) => self.dom.borrow_mut().insert_after(*prev_element, id),
            NodeOrText::AppendText(text) => {
                let detached = self.detach_text(text);
                self.dom.borrow_mut().insert_after(*prev_element, detached);
            }
        }
    }

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        let document = self.dom.borrow().document();
        let doctype = self.dom.borrow_mut().create_detached(NodeData::Doctype);
        self.dom.borrow_mut().append(document, doctype);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        // M2a: template contents stay in place (templates are not rendered).
        // The separate-fragment model arrives with template support.
        *target
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.report.borrow_mut().quirks_mode = mode;
    }

    fn append_before_sibling(&self, sibling: &NodeId, new_node: NodeOrText<NodeId>) {
        match new_node {
            NodeOrText::AppendNode(id) => self.dom.borrow_mut().insert_before(*sibling, id),
            NodeOrText::AppendText(text) => {
                let detached = self.detach_text(text);
                self.dom.borrow_mut().insert_before(*sibling, detached);
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &NodeId, attrs: Vec<Attribute>) {
        let new_attrs: Vec<DomAttribute> = attrs.iter().map(Self::to_dom_attribute).collect();
        let mut dom = self.dom.borrow_mut();
        let Node {
            data:
                NodeData::Element {
                    attrs: existing,
                    fixture_id,
                    ..
                },
            ..
        } = dom.node_mut(*target)
        else {
            return;
        };
        for attr in new_attrs {
            if !existing.iter().any(|a: &DomAttribute| a.name == attr.name) {
                existing.push(attr);
            }
        }
        // Re-extract the fixture id in case data-vv-test arrived late.
        *fixture_id = existing
            .iter()
            .find(|a| a.name == "data-vv-test")
            .map(|a| a.value.clone());
    }

    fn remove_from_parent(&self, target: &NodeId) {
        self.dom.borrow_mut().detach(*target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        self.dom.borrow_mut().reparent_children(*node, *new_parent);
    }
}

/// Parses HTML source (leniently, per the HTML5 tree-construction algorithm).
pub(crate) fn parse(source: &str) -> Dom {
    let sink = DomSink {
        dom: RefCell::new(Dom::empty()),
        names: RefCell::new(HashMap::new()),
        report: RefCell::new(ParseReport {
            parse_errors: 0,
            quirks_mode: QuirksMode::NoQuirks,
        }),
    };
    let parser = html5ever::parse_document(sink, ParseOpts::default());
    use html5ever::tendril::TendrilSink;
    parser.one(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::NodeData;

    /// Finds the first element with the given tag under `root`.
    fn find_tag(dom: &Dom, root: NodeId, tag: &str) -> Option<NodeId> {
        let mut found = None;
        dom.walk_from(root, |id, node| {
            if found.is_none() {
                if let NodeData::Element { name, .. } = &node.data {
                    if name == tag {
                        found = Some(id);
                    }
                }
            }
        });
        found
    }

    #[test]
    fn parses_nesting_and_attributes() {
        let dom = parse("<div class=\"card\" data-vv-test=\"c1\"><p>hi</p></div>");
        let div = find_tag(&dom, dom.document(), "div").expect("div exists");
        assert_eq!(dom.tag_name(div), Some("div"));
        assert_eq!(dom.fixture_id(div), Some("c1"));
        let p = find_tag(&dom, div, "p").expect("p exists");
        assert_eq!(dom.descendant_text(p), "hi");
    }

    #[test]
    fn html_head_body_are_synthesized() {
        let dom = parse("<p>no html tags here</p>");
        let html = find_tag(&dom, dom.document(), "html").expect("html synthesized");
        assert!(find_tag(&dom, html, "body").is_some(), "body synthesized");
    }

    #[test]
    fn adjacent_text_coalesces_but_comments_split_runs() {
        let plain = parse("<p>ab</p>");
        let p = find_tag(&plain, plain.document(), "p").unwrap();
        let text_children = plain
            .node(p)
            .children
            .iter()
            .filter(|c| matches!(plain.node(**c).data, NodeData::Text(_)))
            .count();
        assert_eq!(text_children, 1, "a b → one text node");

        let with_comment = parse("<p>a<!--x-->b</p>");
        let p = find_tag(&with_comment, with_comment.document(), "p").unwrap();
        let text_children = with_comment
            .node(p)
            .children
            .iter()
            .filter(|c| matches!(with_comment.node(**c).data, NodeData::Text(_)))
            .count();
        assert_eq!(
            text_children, 2,
            "comment splits the run like a browser DOM"
        );
        assert_eq!(with_comment.descendant_text(p), "ab");
    }

    #[test]
    fn void_and_implicitly_closed_elements() {
        let dom = parse("<ul><li>one<li>two</ul><br>");
        let mut li_count = 0;
        dom.walk(|_id, node| {
            if let NodeData::Element { name, .. } = &node.data {
                if name == "li" {
                    li_count += 1;
                }
            }
        });
        assert_eq!(li_count, 2, "<li> closes implicitly");
        assert!(
            find_tag(&dom, dom.document(), "br").is_some(),
            "void <br> parsed"
        );
    }

    #[test]
    fn malformed_input_is_recovered() {
        // Unclosed tags, stray text, and bogus attributes must not panic and
        // must still produce usable text.
        let dom = parse("<div><p>oops <b>bold<p>still-b<em>!");
        let text = dom.descendant_text(dom.document());
        assert!(text.contains("oops"));
        assert!(text.contains("bold"));
    }

    #[test]
    fn comments_and_doctype_are_not_elements() {
        let dom = parse("<!doctype html><!--note--><div>x</div>");
        let mut element_count = 0;
        dom.walk(|_id, node| {
            if matches!(node.data, NodeData::Element { .. }) {
                element_count += 1;
            }
        });
        // html + head + body + div (head/body synthesized per the spec).
        assert_eq!(element_count, 4);
    }
}
