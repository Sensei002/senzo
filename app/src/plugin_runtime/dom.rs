//! Minimal HTML tree bridge for the Cheerio subset used by Nuvio providers.

use std::{cell::RefCell, collections::HashMap};

use ego_tree::NodeId;
use scraper::{ElementRef, Html, Selector};
use serde_json::json;

struct Document {
    html: Html,
    nodes: Vec<NodeId>,
    index: HashMap<NodeId, usize>,
}

#[derive(Default)]
pub(super) struct DomStore {
    documents: RefCell<Vec<Document>>,
}

impl DomStore {
    pub(super) fn load(&self, html: &str) -> usize {
        let document = Html::parse_document(html);
        let nodes = document
            .tree
            .nodes()
            .filter(|node| node.value().as_element().is_some())
            .map(|node| node.id())
            .collect::<Vec<_>>();
        let index = nodes
            .iter()
            .enumerate()
            .map(|(position, id)| (*id, position))
            .collect::<HashMap<_, _>>();
        let mut documents = self.documents.borrow_mut();
        documents.push(Document {
            html: document,
            nodes,
            index,
        });
        documents.len()
    }

    pub(super) fn select(&self, document_id: usize, selector: &str) -> String {
        let documents = self.documents.borrow();
        let Some(document) = documents.get(document_id.saturating_sub(1)) else {
            return "[]".to_owned();
        };
        let Ok(selector) = Selector::parse(selector) else {
            return "[]".to_owned();
        };
        let ids = document
            .html
            .select(&selector)
            .filter_map(|element| document.index.get(&element.id()).copied())
            .collect::<Vec<_>>();
        serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_owned())
    }

    pub(super) fn find(&self, document_id: usize, element_id: usize, selector: &str) -> String {
        let documents = self.documents.borrow();
        let Some(document) = documents.get(document_id.saturating_sub(1)) else {
            return "[]".to_owned();
        };
        let Some(element) = element(document, element_id) else {
            return "[]".to_owned();
        };
        let Ok(selector) = Selector::parse(selector) else {
            return "[]".to_owned();
        };
        let ids = element
            .select(&selector)
            .filter_map(|child| document.index.get(&child.id()).copied())
            .collect::<Vec<_>>();
        serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_owned())
    }

    pub(super) fn children(&self, document_id: usize, element_id: usize, selector: &str) -> String {
        let documents = self.documents.borrow();
        let Some(document) = documents.get(document_id.saturating_sub(1)) else {
            return "[]".to_owned();
        };
        let Some(element) = element(document, element_id) else {
            return "[]".to_owned();
        };
        let Ok(selector) = Selector::parse(selector) else {
            return "[]".to_owned();
        };
        let ids = element
            .children()
            .filter_map(ElementRef::wrap)
            .filter(|child| selector.matches(child))
            .filter_map(|child| document.index.get(&child.id()).copied())
            .collect::<Vec<_>>();
        serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_owned())
    }

    pub(super) fn text(&self, document_id: usize, element_id: usize) -> String {
        let documents = self.documents.borrow();
        let Some(document) = documents.get(document_id.saturating_sub(1)) else {
            return String::new();
        };
        element(document, element_id)
            .map(|element| element.text().collect::<String>())
            .unwrap_or_default()
    }

    pub(super) fn inner_html(&self, document_id: usize, element_id: usize) -> String {
        let documents = self.documents.borrow();
        let Some(document) = documents.get(document_id.saturating_sub(1)) else {
            return String::new();
        };
        element(document, element_id)
            .map(|element| element.inner_html())
            .unwrap_or_default()
    }

    pub(super) fn outer_html(&self, document_id: usize, element_id: usize) -> String {
        let documents = self.documents.borrow();
        let Some(document) = documents.get(document_id.saturating_sub(1)) else {
            return String::new();
        };
        if element_id == 0 {
            return document.html.html();
        }
        element(document, element_id)
            .map(|element| element.html())
            .unwrap_or_default()
    }

    pub(super) fn attr(&self, document_id: usize, element_id: usize, name: &str) -> Option<String> {
        let documents = self.documents.borrow();
        let document = documents.get(document_id.saturating_sub(1))?;
        element(document, element_id)
            .and_then(|element| element.value().attr(name).map(str::to_owned))
    }

    pub(super) fn next(&self, document_id: usize, element_id: usize) -> Option<usize> {
        self.sibling(document_id, element_id, true)
    }

    pub(super) fn prev(&self, document_id: usize, element_id: usize) -> Option<usize> {
        self.sibling(document_id, element_id, false)
    }

    fn sibling(&self, document_id: usize, element_id: usize, forward: bool) -> Option<usize> {
        let documents = self.documents.borrow();
        let document = documents.get(document_id.saturating_sub(1))?;
        let mut node = document.html.tree.get(*document.nodes.get(element_id)?)?;
        loop {
            let next = if forward {
                node.next_sibling()
            } else {
                node.prev_sibling()
            }?;
            if next.value().as_element().is_some() {
                return document.index.get(&next.id()).copied();
            }
            node = next;
        }
    }

    pub(super) fn parent(&self, document_id: usize, element_id: usize) -> Option<usize> {
        let documents = self.documents.borrow();
        let document = documents.get(document_id.saturating_sub(1))?;
        let node = document.html.tree.get(*document.nodes.get(element_id)?)?;
        let parent = node.parent()?;
        document.index.get(&parent.id()).copied()
    }

    pub(super) fn is(&self, document_id: usize, element_id: usize, selector: &str) -> bool {
        let documents = self.documents.borrow();
        let Some(document) = documents.get(document_id.saturating_sub(1)) else {
            return false;
        };
        let Some(element) = element(document, element_id) else {
            return false;
        };
        Selector::parse(selector)
            .map(|selector| selector.matches(&element))
            .unwrap_or(false)
    }

    pub(super) fn closest(
        &self,
        document_id: usize,
        element_id: usize,
        selector: &str,
    ) -> Option<usize> {
        let documents = self.documents.borrow();
        let document = documents.get(document_id.saturating_sub(1))?;
        let selector = Selector::parse(selector).ok()?;
        let mut node = document.html.tree.get(*document.nodes.get(element_id)?)?;
        loop {
            if let Some(element) = ElementRef::wrap(node)
                && selector.matches(&element)
            {
                return document.index.get(&element.id()).copied();
            }
            node = node.parent()?;
        }
    }
}

fn element<'a>(document: &'a Document, element_id: usize) -> Option<ElementRef<'a>> {
    let node = document.html.tree.get(*document.nodes.get(element_id)?)?;
    ElementRef::wrap(node)
}

/// Build a JSON value for a parsed URL; kept here so the engine has no direct
/// dependency on browser URL semantics.
pub(super) fn parse_url(value: &str) -> String {
    let Ok(url) = url::Url::parse(value) else {
        return json!({ "error": "invalid url" }).to_string();
    };
    json!({
        "href": url.as_str(),
        "protocol": format!("{}:", url.scheme()),
        "username": url.username(),
        "password": url.password().unwrap_or_default(),
        "host": url.host_str().map(|host| match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        }).unwrap_or_default(),
        "hostname": url.host_str().unwrap_or_default(),
        "port": url.port().map(|port| port.to_string()).unwrap_or_default(),
        "pathname": url.path(),
        "search": url.query().map(|query| format!("?{query}")).unwrap_or_default(),
        "hash": url.fragment().map(|fragment| format!("#{fragment}")).unwrap_or_default(),
        "origin": format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default()),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_and_reads_attributes() {
        let store = DomStore::default();
        let id = store.load("<div class='a'><a href='x'>Hi <b>there</b></a></div>");
        let ids: Vec<usize> = serde_json::from_str(&store.select(id, "div.a a")).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(store.attr(id, ids[0], "href").as_deref(), Some("x"));
        assert_eq!(store.text(id, ids[0]), "Hi there");
        assert!(store.is(id, ids[0], "a"));
    }

    #[test]
    fn parses_relative_urls() {
        let value = parse_url("https://example.com/a?b=1#c");
        assert!(value.contains("\"hostname\":\"example.com\""));
    }
}
