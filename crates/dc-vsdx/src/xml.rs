//! A small reader over the slice of XML the drawing format actually uses.
//!
//! Visio parts are deeply nested but shallow in variety: elements with
//! attributes, a `Cell` element carrying every numeric property, and text. A
//! streaming reader that yields those three things is enough, and avoids
//! pulling a DOM in for documents that can run to thousands of shapes.

use crate::Error;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;

/// One element of a part, with its attributes and its character data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    /// Character data directly inside this element, with the data of any
    /// descendant elements appended in document order.
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every descendant with a name, depth first.
    pub fn descendants<'a>(&'a self, name: &'a str) -> Vec<&'a Node> {
        let mut found = Vec::new();
        let mut stack: Vec<&Node> = self.children.iter().collect();
        while let Some(node) = stack.pop() {
            if node.name == name {
                found.push(node);
            }
            stack.extend(node.children.iter());
        }
        found
    }
}

/// Parses a part into a tree rooted at its document element.
pub fn parse(xml: &str) -> Result<Node, Error> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().expand_empty_elements = false;

    let mut root = Node { name: String::new(), ..Default::default() };
    let mut stack: Vec<Node> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                stack.push(element(name, e.attributes())?);
            }
            Ok(Event::Empty(e)) => {
                let name = e.local_name().as_ref().to_string();
                let node = element(name, e.attributes())?;
                push_child(&mut stack, &mut root, node);
            }
            Ok(Event::End(_)) => {
                if let Some(node) = stack.pop() {
                    push_child(&mut stack, &mut root, node);
                }
            }
            Ok(Event::Text(e)) => {
                let text = e.xml10_content();
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&text);
                }
            }
            Ok(Event::CData(e)) => {
                let text = e.into_inner().into_owned();
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&text);
                }
            }
            // An entity reference arrives as its own event rather than inside
            // the text around it, so it is resolved and appended here.
            Ok(Event::GeneralRef(e)) => {
                let raw = format!("&{};", e.into_inner());
                let resolved = quick_xml::escape::unescape(&raw)
                    .map(|c| c.into_owned())
                    .unwrap_or(raw);
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&resolved);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::Xml(e.to_string())),
            _ => {}
        }
    }

    // Unbalanced markup leaves elements open; close them rather than losing
    // everything the part did say.
    while let Some(node) = stack.pop() {
        push_child(&mut stack, &mut root, node);
    }

    Ok(root)
}

fn push_child(stack: &mut [Node], root: &mut Node, node: Node) {
    // Text inside a child belongs to its ancestors too: Visio splits a label
    // across `<cp>` and `<pp>` run markers, and the label is their
    // concatenation.
    match stack.last_mut() {
        Some(parent) => {
            parent.text.push_str(&node.text);
            parent.children.push(node);
        }
        None => root.children.push(node),
    }
}

fn element(
    name: String,
    attributes: quick_xml::events::attributes::Attributes<'_>,
) -> Result<Node, Error> {
    let mut map = BTreeMap::new();
    for attribute in attributes {
        let attribute = attribute.map_err(|e| Error::Xml(e.to_string()))?;
        // Visio parts declare a default namespace, but relationship parts use
        // `r:` prefixes; matching on the local name handles both without a
        // namespace table.
        let key = attribute.key.local_name().as_ref().to_string();
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::Explicit1_0)
            .map_err(|e| Error::Xml(e.to_string()))?
            .into_owned();
        map.insert(key, value);
    }
    Ok(Node { name, attributes: map, text: String::new(), children: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_attributes_and_text_are_read() {
        let doc = parse(r#"<Shape ID="1" NameU="Process"><Text>Start</Text></Shape>"#).unwrap();
        let shape = &doc.children[0];
        assert_eq!(shape.name, "Shape");
        assert_eq!(shape.attr("ID"), Some("1"));
        assert_eq!(shape.attr("NameU"), Some("Process"));
        assert_eq!(shape.child("Text").unwrap().text, "Start");
    }

    #[test]
    fn empty_elements_become_childless_nodes() {
        let doc = parse(r#"<Shapes><Cell N="PinX" V="4.25"/></Shapes>"#).unwrap();
        let cell = &doc.children[0].children[0];
        assert_eq!(cell.name, "Cell");
        assert_eq!(cell.attr("V"), Some("4.25"));
        assert!(cell.children.is_empty());
    }

    #[test]
    fn namespace_prefixes_are_dropped() {
        let doc = parse(
            r#"<Relationships xmlns:r="x"><Relationship r:id="rId1" Target="page1.xml"/></Relationships>"#,
        )
        .unwrap();
        let rel = &doc.children[0].children[0];
        assert_eq!(rel.name, "Relationship");
        assert_eq!(rel.attr("id"), Some("rId1"));
    }

    #[test]
    fn text_split_across_run_markers_is_rejoined() {
        // Visio writes a styled label as text interleaved with run markers.
        let doc = parse(r#"<Text><cp IX="0"/>Hello <cp IX="1"/>world</Text>"#).unwrap();
        assert_eq!(doc.children[0].text, "Hello world");
    }

    #[test]
    fn entities_are_decoded() {
        let doc = parse(r#"<Text>a &amp; b &lt;c&gt;</Text>"#).unwrap();
        assert_eq!(doc.children[0].text, "a & b <c>");
    }

    #[test]
    fn cdata_is_read_as_text() {
        let doc = parse(r#"<Text><![CDATA[raw < text]]></Text>"#).unwrap();
        assert_eq!(doc.children[0].text, "raw < text");
    }

    #[test]
    fn nesting_is_preserved() {
        let doc = parse(
            r#"<PageContents><Shapes><Shape ID="1"><Shapes><Shape ID="2"/></Shapes></Shape></Shapes></PageContents>"#,
        )
        .unwrap();
        let page = &doc.children[0];
        assert_eq!(page.descendants("Shape").len(), 2);
    }

    #[test]
    fn unbalanced_markup_does_not_lose_everything() {
        let doc = parse(r#"<Shapes><Shape ID="1">"#).unwrap();
        assert_eq!(doc.descendants("Shape").len(), 1);
    }

    #[test]
    fn an_empty_part_parses_to_an_empty_tree() {
        let doc = parse("").unwrap();
        assert!(doc.children.is_empty());
    }

    #[test]
    fn declarations_and_comments_are_skipped() {
        let doc = parse(
            r#"<?xml version="1.0"?><!-- a note --><Shapes><Shape ID="1"/></Shapes>"#,
        )
        .unwrap();
        assert_eq!(doc.children.len(), 1);
        assert_eq!(doc.children[0].name, "Shapes");
    }
}
