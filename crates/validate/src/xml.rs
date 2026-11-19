//! Namespace-aware reader for already decoded XML parts.

use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event},
    name::{Namespace, NamespaceError, NamespaceResolver, ResolveResult},
};

/// Keep the original events for diagnostics, but bind XML-normalized namespace values.
pub(crate) struct XmlReader<'a> {
    reader: Reader<&'a [u8]>,
    resolver: NamespaceResolver,
    pending_pop: bool,
}

impl<'a> XmlReader<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Self { reader: Reader::from_reader(text.as_bytes()), resolver: NamespaceResolver::default(), pending_pop: false }
    }

    pub(crate) fn resolver(&self) -> &NamespaceResolver {
        &self.resolver
    }

    pub(crate) fn read_event_into<'b>(&mut self, buf: &'b mut Vec<u8>) -> quick_xml::Result<Event<'b>> {
        observation_contract! {
            rules: "-";
            collections: "type.Vec*1";
            scope: "one XML event from a byte-bounded decoded part; namespace declarations capped per element";
            loss: "reader failure, namespace binding error or declaration/depth limit";
            fallback: "return the read error to the caller, retaining the already processed event prefix";
            package_loss: "independent";
        }

        if self.pending_pop {
            self.resolver.pop();
        }
        let event = self.reader.read_event_into(buf)?;
        if let Event::Start(e) | Event::Empty(e) = &event {
            self.push(e)?;
        }
        self.pending_pop = matches!(event, Event::Empty(_) | Event::End(_));
        Ok(event)
    }

    pub(crate) fn read_resolved_event_into<'b>(&mut self, buf: &'b mut Vec<u8>) -> quick_xml::Result<(ResolveResult<'_>, Event<'b>)> {
        observation_contract! {
            rules: "-";
            collections: "type.Vec*1";
            scope: "one XML event from a byte-bounded decoded part; namespace declarations capped per element";
            loss: "reader failure, namespace binding error or declaration/depth limit";
            fallback: "return the read error to the caller, retaining the already processed event prefix";
            package_loss: "independent";
        }

        let event = self.read_event_into(buf)?;
        Ok(self.resolver.resolve_event(event))
    }

    fn push(&mut self, element: &BytesStart<'_>) -> quick_xml::Result<()> {
        let level = self.resolver.level().checked_add(1).ok_or_else(|| std::io::Error::other("XML namespace nesting overflow"))?;
        self.resolver.set_level(level);
        let cap = self.resolver.max_declarations_per_element();
        let mut count = 0;
        // Match quick-xml's resolver traversal. The callers check the original attributes
        // for syntax separately, which preserves the diagnostics on a malformed event.
        for attr in element.attributes().with_checks(false) {
            let Ok(attr) = attr else { break };
            let Some(prefix) = attr.key.as_namespace_binding() else { continue };
            if count >= cap {
                return Err(NamespaceError::TooManyDeclarations(cap).into());
            }
            count += 1;
            // Expand each declaration once. Resolving inherited names then borrows the URI
            // regardless of the size of the original spelling or the number of descendants.
            let normalized = attr.normalized_value(XmlVersion::Implicit1_0);
            let value = normalized.as_ref().map_or(attr.value.as_ref(), |uri| uri.as_bytes());
            self.resolver.add(prefix, Namespace(value))?;
        }
        Ok(())
    }
}
