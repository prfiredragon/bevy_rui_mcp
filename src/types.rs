//! Data structures for `bevy_rui_mcp` index records, metadata, configuration, and search hits.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `skip_serializing_if` predicate for the `usize` line-range fields: `0` means "no code range".
/// Serde calls this with a reference to the field, hence the `&usize` parameter.
pub fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}

/// Current persisted index schema version. Bumped to `2` when `RuiRecord` began persisting the
/// code line range, doc-link, widget-provenance, implemented-trait, and example-source fields.
/// v1 caches are still **accepted**: every field added by v2 carries a serde default, so an
/// older cache deserializes and is migrated in memory by `load_index_from_disk`, with the origin
/// reported through `migrated_from_cache_version`. A GitHub bundle published from an older build
/// therefore installs and serves instead of being orphaned; only newer-than-current or invalid
/// schemas are rejected.
pub const CACHE_VERSION: u32 = 2;

/// A single field of a struct or variant of an enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldRecord {
    pub name: String,
    pub ty: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub docs: String,
}

/// An indexed item from `bevy_rui` (`struct`, `enum`, `fn`, `trait`, `trait_method`, `method`, `module`, `example`, or `doc`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuiRecord {
    /// Deterministic unique identifier (e.g. `api:bevy_rui::widgets::button::spawn_button`).
    pub id: String,
    /// Source category (`"ast"`, `"example"`, `"markdown"`).
    pub source: String,
    /// Item kind (`"struct"`, `"enum"`, `"fn"`, `"trait"`, `"trait_method"`, `"method"`, `"module"`, `"example"`, `"guide"`).
    pub kind: String,
    /// Short symbol or example name (e.g. `spawn_button`, `RuiTheme`, `button`, `layout_dock`).
    pub name: String,
    /// Fully qualified path (e.g. `bevy_rui::widgets::button::spawn_button`).
    pub full_path: String,
    /// Module path (e.g. `bevy_rui::widgets::button`).
    pub module: String,
    /// Associated widget name if inside `src/widgets/<widget>.rs` (e.g. `"button"`, `"code_editor"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<String>,
    /// Owning type or trait name for methods/trait methods (e.g. `RuiBuilderExt`, `RuiActiveScope`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Rust item signature or declaration header.
    pub signature: String,
    /// Extracted `///` or `//!` documentation text.
    pub docs: String,
    /// Struct fields or enum variants, if applicable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldRecord>,
    /// Derive macros or key attributes on the type (e.g. `["Component", "Clone"]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derives: Vec<String>,
    /// Extracted `impl Default` or `impl FromWorld` body, if present on this type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_impl: Option<String>,
    /// Full source code or body snippet (for `spawn_*` functions and examples). Never truncated by the
    /// indexer; tools paginate it with `max_bytes`/`code_offset` so a caller can always retrieve the rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// 1-based first line of `code` inside `file` (`0` when `code` is absent).
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub code_start_line: usize,
    /// 1-based last line of `code` inside `file` (`0` when `code` is absent).
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub code_end_line: usize,
    /// Absolute paths or display names this record's documentation links to with `[...]` markdown links,
    /// normalized (`crate::`/`bevy_rui::` prefixes stripped, lowercased). Used to build the reference graph.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc_links: Vec<String>,
    /// How `widget` was associated: `"path"` (declared under `src/widgets/<widget>/`), `"owner"`
    /// (belongs to a type that owns the widget), `"name"` (builder/spawn method named after the widget),
    /// `"reexport"` (brought into scope by a `pub use`), `"doc_link"` (named by the item's own
    /// `/// See [`<widget>::spawn_...`]` markdown links, matched against widget names declared by the
    /// path rule), or `"decl_trait"` (inherited from the trait declaration that declares this method,
    /// so an `impl` body carrying no doc comments still files under its widget). Each value makes the
    /// tagging verifiable; records whose widget is only asserted, never evidenced, stay `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_from: Option<String>,
    /// Traits the record participates in, extracted from `impl Trait for Type` blocks and trait
    /// declarations. On `struct`/`enum` records: traits implemented by that type (e.g. `Plugin`).
    /// On `trait_method` records: the trait declaration that contains the method. On `method`
    /// records: the traits inherited from the owning type's `implements` list. `Default`/`FromWorld`
    /// bodies are captured in `default_impl` instead of here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implements: Vec<String>,
    /// Original `name.rs` stem for example-derived records.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub example_source: String,
    /// Relative file path inside `bevy_rui` (e.g. `src/widgets/button.rs`).
    pub file: String,
    /// 1-based start line in `file`.
    pub line: usize,
    /// `bevy_rui` version this record was indexed from.
    pub version: String,
}

/// Persisted metadata for a built `bevy_rui` version index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexMetadata {
    pub cache_version: u32,
    pub rui_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevy_version: Option<String>,
    pub built_at: String,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Populated in memory when an index was built from a local checkout. Never persisted or shipped
    /// (`save_index_to_disk` and `publish_index_bundle` clear it): it is a machine-local absolute
    /// path, and persisted indexes plus release bundles are committed to this repo and downloaded
    /// by unrelated projects, so shipped data identifies the index by `rui_version` alone.
    pub rui_path: Option<String>,
    pub total_records: usize,
    pub total_symbols: usize,
    /// Number of doc-link edges discovered across all records (`sum(len(doc_links))`).
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub total_doc_links: usize,
    /// Number of implemented-trait edges discovered across all records (`sum(len(implements))`).
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub total_implements: usize,
    pub widgets: Vec<String>,
    pub by_kind: BTreeMap<String, usize>,
    /// Set when this index was loaded from an older `cache_version` and migrated in memory by
    /// `load_index_from_disk`; `None` when built fresh or loaded from the current schema. Lets
    /// `rui_status` report that a downloaded bundle came from an older publisher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrated_from_cache_version: Option<u32>,
}

/// Entry stored in `data/registry.json` tracking installed versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub version: String,
    pub cache_version: u32,
    pub built_at: String,
    pub records: usize,
    pub symbols: usize,
    pub widgets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Kept for compatibility with older `registry.json` shapes; current writers store `None` here so
    /// the registry never records another machine's source path (see `save_index_to_disk`).
    pub rui_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// Root structure of `data/registry.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryData {
    pub cache_version: u32,
    pub versions: BTreeMap<String, RegistryEntry>,
}

impl Default for RegistryData {
    fn default() -> Self {
        Self {
            cache_version: CACHE_VERSION,
            versions: BTreeMap::new(),
        }
    }
}

/// Manifest generated by `publish-index` and read by `fetch-index`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub manifest_version: u32,
    pub rui_version: String,
    pub asset: String,
    pub revision: u32,
    pub sha256: String,
    pub cache_version: u32,
    pub size: u64,
    pub total_records: usize,
    pub built_at: String,
}

/// Ranked search result returned by hybrid search.
#[derive(Debug, Clone)]
pub struct SearchHit<'a> {
    pub record: &'a RuiRecord,
    pub score: f64,
    /// Which part of the record produced the score (`symbol`, `name`, `docs`, or `symbol + docs`), so
    /// a caller can tell an exact symbol hit from a loose documentation hit.
    pub reason: String,
}
