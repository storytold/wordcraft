//! Namespace-aware reader for already decoded XML parts.

use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event},
    name::{LocalName, Namespace, NamespaceError, NamespaceResolver, PrefixDeclaration, QName, ResolveResult},
};

/// The binding recorded for a default namespace declaration whose value could not be resolved.
/// The NUL byte cannot occur in a namespace URI, so it never equals one. Resolve elements through
/// [`resolve_element`], which reports it as unknown.
const UNRESOLVED_DEFAULT: &[u8] = b"\0unresolved";

/// Resolve an element name. A default namespace that could not be resolved is unknown, never unbound.
pub(crate) fn resolve_element<'r, 'n>(res: &'r NamespaceResolver, name: QName<'n>) -> (ResolveResult<'r>, LocalName<'n>) {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "one element name resolved against the in-scope namespace bindings";
        loss: "a namespace declaration that could not be resolved";
        fallback: "report the name's namespace as unknown, never unbound or bound";
        package_loss: "independent";
    }

    match res.resolve_element(name) {
        (ResolveResult::Bound(Namespace(UNRESOLVED_DEFAULT)), local) => (ResolveResult::Unknown(Vec::new()), local),
        resolved => resolved,
    }
}

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
        let namespace = match &event {
            Event::Start(e) | Event::Empty(e) => resolve_element(&self.resolver, e.name()).0,
            Event::End(e) => resolve_element(&self.resolver, e.name()).0,
            _ => ResolveResult::Unbound,
        };
        Ok((namespace, event))
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
            // A value that cannot be fully resolved (an entity or character reference that does not
            // expand, bad encoding) is not a namespace. Its spelling is never bound: a prefix becomes
            // unknown, so consumers can neither match nor refute a URI with it.
            let normalized = attr.normalized_value(XmlVersion::Implicit1_0);
            let value: &[u8] = match (&normalized, &prefix) {
                (Ok(uri), _) => uri.as_bytes(),
                (Err(_), PrefixDeclaration::Default) => UNRESOLVED_DEFAULT,
                (Err(_), PrefixDeclaration::Named(_)) => b"",
            };
            self.resolver.add(prefix, Namespace(value))?;
        }
        Ok(())
    }
}
