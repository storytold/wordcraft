//! Relationship consumers in XML, including picture alternatives skipped by the story checks.
use std::collections::HashMap;

use quick_xml::events::attributes::Attribute;
use quick_xml::name::{Namespace, NamespaceResolver, QName, ResolveResult};

use crate::package::{self, Package, RelationshipBindings};
use crate::scan::{RelId, attr_ns, attr_plain};
use crate::{AlternateAssessment, Limits, NS_DS, NS_MC, NS_R, NS_W, NS_W15, Resolution, codecs, decode_alternate_image};

const DRAWING: &[u8] = b"http://schemas.openxmlformats.org/drawingml/2006/main";
const VML: &[u8] = b"urn:schemas-microsoft-com:vml";
const VML_OFFICE: &[u8] = b"urn:schemas-microsoft-com:office:office";
const STRICT_REL: &[u8] = b"http://purl.oclc.org/ooxml/officeDocument/relationships";

/// The enclosing compatibility block, direct branch and base image for one XML element.
#[derive(Clone, Copy, Default)]
pub(crate) struct Scope {
    block: Option<usize>,
    branch: Option<usize>,
    picture: Option<usize>,
    direct_block_child: bool,
}

#[derive(Default)]
struct Block {
    fallback: Option<usize>,
    ambiguous: bool,
}
struct Branch {
    block: usize,
    choice: bool,
    eligible: bool,
    pictures: Vec<usize>,
}
struct Picture {
    branch: Option<usize>,
    embedded: Option<RelId>,
}
struct Use {
    picture: Option<usize>,
    /// An optional extension or link can use the embedded image on the same blip.
    alternative_to_embed: bool,
}

/// Source-local evidence. A capped or disrupted scan cannot prove complete substitution.
#[derive(Default)]
pub(crate) struct Uses {
    blocks: Vec<Block>,
    branches: Vec<Branch>,
    pictures: Vec<Picture>,
    sites: HashMap<RelId, Vec<Use>>,
    site_count: usize,
    pub(crate) binding_ids: Vec<String>,
    pub(crate) store_id: Option<String>,
    pub(crate) uncertain_binding: bool,
    pub(crate) capped: bool,
}

impl Uses {
    pub(crate) fn charge_package_budget(&mut self, remaining: &mut usize) {
        let facts = self.len();
        if facts > *remaining {
            *self = Self { capped: true, ..Self::default() };
        } else {
            *remaining -= facts;
        }
    }

    pub(crate) fn len(&self) -> usize {
        observation_contract! {
            rules: "-";
            collections: "field.binding_ids*1,field.blocks*1,field.branches*1,field.pictures*1";
            scope: "One XML source allocation counter";
            loss: "Source or package fact cap";
            fallback: "Return the fact count charged to the package budget";
            package_loss: "independent";
        }

        self.blocks.len() + self.branches.len() + self.pictures.len() + self.site_count + self.binding_ids.len()
    }

    pub(crate) fn open(
        &mut self,
        parent: Scope,
        element: (Option<&[u8]>, &str, bool),
        attrs: &[Attribute],
        resolver: &NamespaceResolver,
        cap: usize,
    ) -> Scope {
        observation_contract! {
            rules: "-";
            collections: "field.blocks*1,field.branches*1,field.pictures*4,field.sites*1";
            scope: "Namespace-qualified references and picture slots in one XML event";
            loss: "Unresolved attribute or source-local allocation cap";
            fallback: "Retain the observed prefix; incomplete sources do not prove substitution";
            package_loss: "independent";
        }

        let mut scope = Scope { direct_block_child: false, ..parent };
        let (ns, local, root) = element;
        if ns == Some(NS_MC.as_bytes()) {
            scope = self.compatibility_scope(parent, local, attrs, resolver, cap);
        }
        if parent.direct_block_child
            && !(ns == Some(NS_MC.as_bytes()) && matches!(local, "Choice" | "Fallback"))
            && let Some(block) = parent.block
        {
            self.blocks[block].ambiguous = true;
        }
        self.observe_binding((ns, local, root), attrs, resolver, cap);
        let element = (ns, local);
        let drawing = matches!(element, (Some(DRAWING), "blip"));
        let vml = matches!(element, (Some(VML), "imagedata"));
        let primary = drawing || vml;
        let optional = matches!(
            element,
            (Some(b"http://schemas.microsoft.com/office/drawing/2010/main"), "imgLayer")
                | (Some(b"http://schemas.microsoft.com/office/drawing/2016/SVG/main"), "svgBlip")
        );
        if primary {
            if !self.reserve(cap) {
                return scope;
            }
            let index = self.pictures.len();
            if let Some(branch) = scope.branch {
                self.branches[branch].pictures.push(index);
            }
            self.pictures.push(Picture { branch: scope.branch, embedded: None });
            scope.picture = Some(index);
        }
        for attr in attrs {
            let name = attr.key.local_name();
            let name = name.as_ref();
            let ResolveResult::Bound(Namespace(namespace)) = resolver.resolve_attribute(attr.key).0 else { continue };
            let legacy = namespace == VML_OFFICE && name == b"relid";
            if namespace != NS_R.as_bytes() && namespace != STRICT_REL && !legacy {
                continue;
            }
            let Ok(id) = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0) else { continue };
            if !self.reserve(cap) {
                break;
            }
            let embedded = (drawing && name == b"embed") || (vml && (legacy || name == b"id"));
            let alternative_to_embed = ((drawing || optional) && name == b"link") || (optional && name == b"embed");
            let picture = if embedded || alternative_to_embed { scope.picture } else { None };
            if let Some(index) = picture
                && embedded
            {
                self.pictures[index].embedded = Some(id.as_ref().into());
            }
            self.sites.entry(id.into_owned().into()).or_default().push(Use { picture, alternative_to_embed });
            self.site_count += 1;
        }
        scope
    }

    fn reserve(&mut self, cap: usize) -> bool {
        if self.len() >= cap {
            self.capped = true;
        }
        !self.capped
    }

    fn observe_binding(&mut self, (ns, local, root): (Option<&[u8]>, &str, bool), attrs: &[Attribute], resolver: &NamespaceResolver, cap: usize) {
        observation_contract! {
            rules: "-";
            collections: "field.binding_ids*1";
            scope: "WordprocessingML store bindings and custom XML item-properties roots";
            loss: "Unresolved namespace or binding ID, or source-local allocation cap";
            fallback: "Retain positive matches and withhold uncertain binding absence";
            package_loss: "independent";
        }

        if local == "dataBinding" {
            let namespace = match ns {
                Some(ns) if ns == NS_W.as_bytes() => NS_W,
                Some(ns) if ns == NS_W15.as_bytes() => NS_W15,
                None => {
                    self.uncertain_binding = true;
                    return;
                }
                _ => return,
            };
            if let Some(id) = attr_ns(attrs, resolver, namespace, b"storeItemID").or_else(|| attr_ns(attrs, resolver, NS_W, b"storeItemID")) {
                if self.reserve(cap) {
                    self.binding_ids.push(id.trim().trim_matches(['{', '}']).to_ascii_lowercase());
                }
            } else {
                self.uncertain_binding = true;
            }
        } else if root && local == "datastoreItem" && ns == Some(NS_DS.as_bytes()) {
            self.store_id = attr_ns(attrs, resolver, NS_DS, b"itemID").map(|id| id.trim().trim_matches(['{', '}']).to_ascii_lowercase());
        }
    }

    fn compatibility_scope(&mut self, parent: Scope, local: &str, attrs: &[Attribute], resolver: &NamespaceResolver, cap: usize) -> Scope {
        observation_contract! {
            rules: "-";
            collections: "access.prefix*1,field.blocks*6,field.branches*3,type.Vec*1";
            scope: "Direct Choice and Fallback siblings of a namespace-qualified compatibility block";
            loss: "Ambiguous nesting, a Choice after Fallback, or source-local allocation cap";
            fallback: "Only complete, ordered sibling alternatives qualify for substitution";
            package_loss: "independent";
        }

        if local == "AlternateContent" && self.reserve(cap) {
            if let Some(branch) = parent.branch {
                // A nested alternate is assessed independently. Its parent branch cannot count its
                // pictures as unconditional replacements.
                self.branches[branch].eligible = false;
            }
            let index = self.blocks.len();
            self.blocks.push(Block::default());
            return Scope { block: Some(index), branch: None, picture: parent.picture, direct_block_child: true };
        }
        let mut scope = Scope { direct_block_child: false, ..parent };
        if parent.direct_block_child
            && let Some(block) = parent.block
            && matches!(local, "Choice" | "Fallback")
            && self.reserve(cap)
        {
            let choice = local == "Choice";
            let index = self.branches.len();
            let eligible = if choice && self.blocks[block].fallback.is_some() {
                self.blocks[block].ambiguous = true;
                false
            } else if choice {
                attr_plain(attrs, b"Requires").is_some_and(|requires| {
                    !requires.trim().is_empty()
                        && requires.split_ascii_whitespace().all(|prefix| {
                            matches!(resolver.resolve_prefix(QName(format!("{prefix}:requires").as_bytes()).prefix(), false), ResolveResult::Bound(_))
                        })
                })
            } else {
                if self.blocks[block].fallback.replace(index).is_some() {
                    self.blocks[block].ambiguous = true;
                }
                true
            };
            self.branches.push(Branch { block, choice, eligible, pictures: Vec::new() });
            scope.branch = Some(index);
        }
        scope
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Consumption {
    #[default]
    Unused,
    Substituted,
    Picture,
    Other,
}

/// A package-wide decoder budget and cache for proving that replacement pictures render.
pub(crate) struct FallbackImages<'a> {
    pkg: &'a Package,
    per_part: u64,
    remaining: u64,
    image_cache: HashMap<&'a str, bool>,
}
impl<'a> FallbackImages<'a> {
    pub(crate) fn new(pkg: &'a Package, limits: Limits) -> Self {
        observation_contract! {
            rules: "-";
            collections: "type.HashMap*1";
            scope: "Package-wide cache and bounded decode budget for candidate replacement images";
            loss: "Decoder, package or allocation budget unavailable";
            fallback: "No rendering claim without positive raster evidence";
            package_loss: "independent";
        }

        Self { pkg, per_part: limits.part_bytes, remaining: limits.total_bytes, image_cache: HashMap::new() }
    }

    fn renders(&mut self, bindings: &RelationshipBindings<'a>, id: &RelId) -> bool {
        observation_contract! {
            rules: "-";
            collections: "field.bytes*1,field.content_types*1,field.image_cache*1,lookup.entry_by_name*1";
            scope: "First-bound internal image with an image type and successfully decoded raster content";
            loss: "Unavailable or malformed target, unsupported image format or decode budget";
            fallback: "Retain findings when substitute rendering is unproved";
            package_loss: "independent";
        }

        let Resolution::One(rel) = bindings.resolve(id.as_str()) else { return false };
        let Some(target) = rel.target_abs.as_deref().filter(|_| rel.role.kind() == Some("image")) else { return false };
        *self.image_cache.entry(target).or_insert_with(|| {
            if !self.pkg.content_types.of(target).is_some_and(|ct| ct.starts_with("image/")) {
                return false;
            }
            let Resolution::One(entry) = package::entry_by_name(self.pkg, target) else { return false };
            let Some(bytes) = entry.bytes.as_deref() else { return false };
            let Some(format) = codecs::detect(bytes) else { return false };
            matches!(decode_alternate_image(format, bytes, self.per_part, &mut self.remaining), Ok(AlternateAssessment::Usable))
        })
    }
}

pub(crate) struct Consumers<'a> {
    facts: &'a Uses,
    substituted_branches: HashMap<usize, bool>,
    complete: bool,
}
impl<'a> Consumers<'a> {
    pub(crate) fn new(facts: &'a Uses, complete: bool) -> Self {
        observation_contract! {
            rules: "-";
            collections: "type.HashMap*1";
            scope: "One source and a cache of its established fallback branches";
            loss: "Incomplete XML syntax or source-local allocation cap";
            fallback: "Retain observed uses without proving substitution";
            package_loss: "independent";
        }

        Self { facts, substituted_branches: HashMap::new(), complete: complete && !facts.capped }
    }

    pub(crate) fn consumption<'p>(&mut self, id: &RelId, bindings: &RelationshipBindings<'p>, images: &mut FallbackImages<'p>) -> Consumption {
        observation_contract! {
            rules: "-";
            collections: "field.pictures*1,field.sites*1,field.substituted_branches*1";
            scope: "All observed occurrences of a lexical relationship ID in one source";
            loss: "Incomplete source or unproved image fallback";
            fallback: "A regular use or unsubstituted picture keeps the target obligation";
            package_loss: "independent";
        }

        let Some(sites) = self.facts.sites.get(id) else { return Consumption::Unused };
        let mut result = Consumption::Substituted;
        for site in sites {
            let Some(index) = site.picture else { return Consumption::Other };
            let picture = &self.facts.pictures[index];
            if self.complete {
                let replaced = site.alternative_to_embed && picture.embedded.as_ref().is_some_and(|id| images.renders(bindings, id));
                if replaced {
                    continue;
                }
                if let Some(branch) = picture.branch
                    && *self.substituted_branches.entry(branch).or_insert_with(|| self.facts.substitutes(branch, bindings, images))
                {
                    continue;
                }
            }
            result = Consumption::Picture;
        }
        result
    }
}

impl Uses {
    pub(crate) fn uses(&self, id: &RelId) -> bool {
        observation_contract! {
            rules: "-";
            collections: "field.sites*1";
            scope: "Positive relationship-reference facts from one XML source";
            loss: "Incomplete syntax or allocation";
            fallback: "Only observed uses establish consumption";
            package_loss: "independent";
        }

        self.sites.contains_key(id)
    }

    fn substitutes<'p>(&self, branch: usize, bindings: &RelationshipBindings<'p>, images: &mut FallbackImages<'p>) -> bool {
        observation_contract! {
            rules: "-";
            collections: "field.blocks*1,field.branches*2,field.pictures*4";
            scope: "All picture slots of the direct fallback sibling of this choice";
            loss: "Incomplete or ambiguous branch, unequal picture counts, or unusable target";
            fallback: "Suppress only when the complete fallback renders every picture slot";
            package_loss: "independent";
        }

        let choice = &self.branches[branch];
        let block = &self.blocks[choice.block];
        if !choice.choice || !choice.eligible || block.ambiguous {
            return false;
        }
        let Some(fallback) = block.fallback.map(|index| &self.branches[index]) else { return false };
        fallback.eligible
            && fallback.pictures.len() == choice.pictures.len()
            && fallback.pictures.iter().all(|&index| self.pictures[index].embedded.as_ref().is_some_and(|id| images.renders(bindings, id)))
    }
}
