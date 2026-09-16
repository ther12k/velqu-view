//! Source identity and host-side asset resolution.
//!
//! M1.1 hardening (ADR 0004): content crossing into VelquView carries an
//! identity so later milestones can do something with it — M2 resolves
//! relative image references against a document base, M3 diagnostics report
//! which stylesheet offended, and M6 hot reload *replaces* a sheet instead of
//! appending a doppelgänger.
//!
//! Asset resolution is **host-provided**: the core renderer performs no
//! filesystem or network I/O of its own. A host (such as `velqu-lab`) installs
//! an [`AssetResolver`]; the default resolves nothing.

use std::fmt;
use std::rc::Rc;

/// Stable identity for a loaded source (document or stylesheet).
///
/// Typically a file path or URL-ish string chosen by the host; VelquView
/// treats it as opaque. Equality on [`SourceId`] is what makes stylesheet
/// replacement and diagnostics unambiguous.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(String);

impl SourceId {
    /// Creates an id from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An HTML document plus its identity and logical base location.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentSource {
    /// Host-chosen identity (e.g. the file path).
    pub id: SourceId,
    /// The HTML source text.
    pub html: String,
    /// Opaque base (e.g. containing directory) handed to the
    /// [`AssetResolver`] when resolving relative references. VelquView never
    /// interprets it as a filesystem path itself.
    pub base: Option<String>,
}

impl DocumentSource {
    /// A document with `id`, `html`, and no base.
    pub fn new(id: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            id: SourceId::new(id),
            html: html.into(),
            base: None,
        }
    }

    /// Attaches a logical base for relative asset resolution.
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self
    }
}

/// A CSS stylesheet plus its identity.
#[derive(Debug, Clone, PartialEq)]
pub struct StylesheetSource {
    /// Host-chosen identity; loading another sheet with the same id replaces
    /// this one (the hot-reload primitive).
    pub id: SourceId,
    /// The CSS source text.
    pub css: String,
}

impl StylesheetSource {
    /// A stylesheet with `id` and `css`.
    pub fn new(id: impl Into<String>, css: impl Into<String>) -> Self {
        Self {
            id: SourceId::new(id),
            css: css.into(),
        }
    }
}

/// A request to resolve one relative asset reference from a document.
///
/// Produced by the renderer (M2+: `<img src>` and friends), answered by the
/// installed [`AssetResolver`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRequest<'a> {
    /// The document's logical base, if it had one.
    pub base: Option<&'a str>,
    /// The reference as written in the document (e.g. `icons/x.svg`).
    pub path: &'a str,
}

/// A resolved asset: raw bytes plus the id it was loaded under.
///
/// Format detection (PNG/SVG/…) is the renderer's job; the resolver only
/// fetches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// Host-chosen identity for the loaded asset (path or synthetic id).
    pub id: SourceId,
    /// Raw asset bytes.
    pub bytes: Vec<u8>,
}

/// Host-provided asset lookup.
///
/// The core renderer has **no ambient I/O**: it never touches the filesystem,
/// network, or anything else directly. A host that wants assets (images,
/// fonts) installs a resolver here; resolution requests flow out through
/// [`AssetRequest`] and bytes flow back through [`Asset`].
pub trait AssetResolver {
    /// Resolves `request` to asset bytes, or `None` when unavailable.
    fn resolve(&self, request: AssetRequest<'_>) -> Option<Asset>;
}

/// The default resolver: resolves nothing (NullHost behavior).
#[derive(Debug, Clone, Copy, Default)]
pub struct NullAssetResolver;

impl AssetResolver for NullAssetResolver {
    fn resolve(&self, _request: AssetRequest<'_>) -> Option<Asset> {
        None
    }
}

/// The renderer's installed resolver; `Rc` because the M1 stack is
/// single-threaded (winit) and hosts may share state cheaply.
pub type SharedAssetResolver = Rc<dyn AssetResolver>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn source_ids_compare_and_display() {
        let a = SourceId::new("app.css");
        assert_eq!(a, SourceId::new("app.css"));
        assert_ne!(a, SourceId::new("index.html"));
        assert_eq!(a.to_string(), "app.css");
        assert_eq!(a.as_str(), "app.css");
    }

    #[test]
    fn document_source_builders() {
        let doc = DocumentSource::new("index.html", "<html></html>").with_base("apps/demo");
        assert_eq!(doc.id.as_str(), "index.html");
        assert_eq!(doc.base.as_deref(), Some("apps/demo"));
        let plain = DocumentSource::new("x", "y");
        assert_eq!(plain.base, None);
    }

    struct RecordingResolver {
        seen: RefCell<Vec<(Option<String>, String)>>,
    }

    impl AssetResolver for RecordingResolver {
        fn resolve(&self, request: AssetRequest<'_>) -> Option<Asset> {
            self.seen
                .borrow_mut()
                .push((request.base.map(str::to_owned), request.path.to_owned()));
            Some(Asset {
                id: SourceId::new(request.path),
                bytes: vec![1, 2, 3],
            })
        }
    }

    #[test]
    fn null_resolver_resolves_nothing() {
        assert!(
            NullAssetResolver
                .resolve(AssetRequest {
                    base: None,
                    path: "a.png"
                })
                .is_none()
        );
    }

    #[test]
    fn resolver_receives_document_base() {
        let resolver = Rc::new(RecordingResolver {
            seen: RefCell::new(Vec::new()),
        });
        // The VelquView wiring is exercised in lib.rs tests; here we pin the
        // trait contract itself: base and path arrive unmodified.
        let out = resolver.resolve(AssetRequest {
            base: Some("apps/demo"),
            path: "icons/x.svg",
        });
        assert_eq!(out.map(|a| a.bytes), Some(vec![1, 2, 3]));
        assert_eq!(
            resolver.seen.borrow().as_slice(),
            [(Some("apps/demo".into()), "icons/x.svg".into())]
        );
    }
}
