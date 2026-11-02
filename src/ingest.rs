//! Direct Rust AST (`syn`) and example/markdown indexer for `bevy_rui`.
//!
//! Parses `src/**/*.rs`, `examples/**/*.rs`, and `README.md` directly without requiring `cargo doc`.

use crate::types::{FieldRecord, RuiRecord};
use quote::ToTokens;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use syn::{Attribute, Expr, Fields, ImplItem, Item, Lit, Meta, ReturnType, TraitItem, Visibility};
use walkdir::WalkDir;

/// Computes a deterministic SHA-256 fingerprint over all `.rs`, `.md`, and `Cargo.toml` files in `rui_dir`.
pub fn compute_source_fingerprint(rui_dir: &Path) -> String {
    let mut hasher = Sha256::new();
    let mut files: Vec<PathBuf> = Vec::new();

    for sub in ["src", "examples"] {
        let dir = rui_dir.join(sub);
        if dir.is_dir() {
            for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file()
                    && entry.path().extension().and_then(|s| s.to_str()) == Some("rs")
                {
                    files.push(entry.path().to_path_buf());
                }
            }
        }
    }
    for extra in ["Cargo.toml", "README.md"] {
        let p = rui_dir.join(extra);
        if p.is_file() {
            files.push(p);
        }
    }

    files.sort();
    for file in files {
        if let Ok(rel) = file.strip_prefix(rui_dir) {
            hasher.update(rel.to_string_lossy().as_bytes());
            hasher.update(b"\0");
        }
        if let Ok(bytes) = fs::read(&file) {
            hasher.update(&bytes);
            hasher.update(b"\0");
        }
    }
    hex::encode(hasher.finalize())
}

/// Extracts `///` and `//!` doc attributes from a `syn::Attribute` slice.
fn extract_docs(attrs: &[Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let Meta::NameValue(nv) = &attr.meta {
                if let Expr::Lit(expr_lit) = &nv.value {
                    if let Lit::Str(s) = &expr_lit.lit {
                        let raw = s.value();
                        let trimmed = raw.strip_prefix(' ').unwrap_or(&raw);
                        lines.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    lines.join("\n").trim().to_string()
}

/// Extracts `#[derive(...)]` macro names from attributes.
fn extract_derives(attrs: &[Attribute]) -> Vec<String> {
    let mut out = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("derive") {
            let tokens = attr.meta.to_token_stream().to_string();
            if let Some(inner) = tokens.strip_prefix("derive").and_then(|s| {
                let s = s.trim();
                s.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
            }) {
                for part in inner.split(',') {
                    let clean = part.trim();
                    if !clean.is_empty() {
                        out.push(clean.to_string());
                    }
                }
            }
        }
    }
    out
}

// The attach pass below restricts `default_impl`/`impl_traits` to struct/enum/trait records, so no
// separate "does this type have a record?" scan is needed here (it was O(n^2) per impl).
fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// True for ASCII letters only (avoids relying on non-portable char predicates).
fn is_ascii_letter(c: char) -> bool {
    (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
}

/// True when `t` looks like an identifier or qualified path (a plausible Rust symbol), so prose
/// link text and URLs never enter the doc-link graph.
fn is_link_target(t: &str) -> bool {
    let first: Option<char> = t.chars().next();
    let head_ok: bool = if let Some(c) = first { is_ascii_letter(c) || c == '_' } else { false };
    head_ok
        && t.chars().all(|c| is_ascii_letter(c) || c.is_ascii_digit() || c == '_' || c == ':')
}

/// Normalizes a doc-link target: strips `crate::`/`bevy_rui::` prefixes and lowercases.
fn normalize_link(t: &str) -> String {
    let s: &str = t.trim();
    let s2: &str = if s.starts_with("crate::") {
        &s["crate::".len()..]
    } else if s.starts_with("bevy_rui::") {
        &s["bevy_rui::".len()..]
    } else {
        s
    };
    s2.to_lowercase()
}

/// Extracts markdown link targets (`[`Sym`]` or `[Sym](url)`) from documentation text and normalizes
/// them, dropping anything that is not identifier-shaped (prose, URLs, images with empty alt text).
fn extract_doc_links(docs: &str) -> Vec<String> {
    let chars: Vec<char> = docs.chars().collect();
    let n: usize = chars.len();
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut i: usize = 0;
    while i < n {
        if chars[i] == '[' {
            let mut j: usize = i + 1;
            let mut inner: String = String::new();
            while j < n && chars[j] != ']' && chars[j] != '[' {
                inner.push(chars[j]);
                j += 1;
            }
            if j < n && chars[j] == ']' {
                let target: String = inner.replace("`", "").trim().to_string();
                if is_link_target(&target) {
                    let normalized: String = normalize_link(&target);
                    if !normalized.is_empty() {
                        found.insert(normalized);
                    }
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    found.iter().cloned().collect()
}

fn clean_tokens(ts: impl ToTokens) -> String {
    let raw = ts.to_token_stream().to_string();
    raw.replace(" :: ", "::")
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace("< ", "<")
        .replace(" , ", ", ")
        .replace(" & ", " &")
        .replace("& mut ", "&mut ")
        .replace(" 'w", "'w")
        .replace(" 'a", "'a")
        .replace("'_ ", "'_")
        .replace(" ( ", "(")
        .replace(" ) ", ")")
        .replace(" )", ")")
        .replace("( ", "(")
        .replace(" : ", ": ")
        .replace(" -> ", " -> ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_fn_signature(sig: &syn::Signature, is_public: bool) -> String {
    let vis_prefix = if is_public { "pub " } else { "" };
    let generics = if sig.generics.params.is_empty() {
        String::new()
    } else {
        clean_tokens(&sig.generics)
    };
    let inputs = sig
        .inputs
        .iter()
        .map(clean_tokens)
        .collect::<Vec<_>>()
        .join(", ");
    let ret = match &sig.output {
        ReturnType::Default => String::new(),
        ReturnType::Type(_, ty) => format!(" -> {}", clean_tokens(ty)),
    };
    format!("{vis_prefix}fn {}{generics}({inputs}){ret}", sig.ident)
}

/// Maps a relative file path under `src/` to `(module_path, optional_widget_name)`.
fn module_and_widget_from_rel(rel: &str) -> (String, Option<String>) {
    let norm = rel.replace('\\', "/");
    let without_src = norm.strip_prefix("src/").unwrap_or(&norm);
    let without_ext = without_src.strip_suffix(".rs").unwrap_or(without_src);

    if without_ext == "lib" {
        return ("bevy_rui".to_string(), None);
    }

    let parts: Vec<&str> = without_ext
        .split('/')
        .filter(|&p| p != "mod")
        .collect();

    let module = if parts.is_empty() {
        "bevy_rui".to_string()
    } else {
        format!("bevy_rui::{}", parts.join("::"))
    };

    let widget = if parts.len() >= 2 && parts[0] == "widgets" {
        Some(parts[1].to_string())
    } else {
        None
    };

    (module, widget)
}

/// The widget names this checkout declares through the file path rule: every name that
/// `module_and_widget_from_rel` assigns to at least one source file under `src/widgets/` (e.g.
/// `button`, `windows`, `color_picker`).
///
/// Builder methods (`pub trait RuiBuilderExt { fn button(...); }` in `src/widgets/mod.rs`) live in one
/// shared file instead of a per-widget file, so the path rule leaves them unassociated and no widget
/// guide can find them. Their documentation states the entry point they spawn — ``See [`windows::spawn_window`]``
/// for `fn window` — so the name before the last `::` in that link is matched against this set. Matching
/// against the real declared names, rather than against the method name, is what maps `fn window` to the
/// `windows` widget instead of inventing a `window` widget that has no directory here.
pub fn widget_names_in_checkout(rui_dir: &Path) -> BTreeSet<String> {
    let src_dir = rui_dir.join("src");
    let mut found: BTreeSet<String> = BTreeSet::new();
    if src_dir.is_dir() {
        let mut rs_files: Vec<PathBuf> = WalkDir::new(&src_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_file()
                    && e.path().extension().and_then(|s| s.to_str()) == Some("rs")
            })
            .map(|e| e.path().to_path_buf())
            .collect();
        rs_files.sort();
        for file_path in &rs_files {
            let rel_file = file_path
                .strip_prefix(rui_dir)
                .unwrap_or(&file_path)
                .to_string_lossy()
                .replace('\\', "/");
            let (_module, widget) = module_and_widget_from_rel(&rel_file);
            if let Some(w) = &widget {
                found.insert(w.to_lowercase());
            }
        }
    }
    found
}

/// Normalizes a singular/plural pair so a documented name and a declared widget name can be compared
/// (`window` vs `windows`). A name is merged only when one is the other minus a trailing `s`; nothing
/// is merged on a fuzzy match.
fn normalize_widget_term(t: &str) -> String {
    let s: String = t.to_lowercase();
    if s.len() > 3usize && s.ends_with("s") && !s.ends_with("ss") {
        s[..s.len() - 1usize].to_string()
    } else {
        s
    }
}

/// Associates a builder method with a widget, returning `(widget, how)` where `how` is `"doc_link"`
/// when the association came from the method's documentation, or an empty string when there is none.
///
/// The only evidence consulted is the method's own doc-link targets. A target such as `layout::spawn_vbox`
/// (for `fn vbox`) or `windows::spawn_window` (for `fn window`) contributes its leading segment when that
/// segment names a widget declared by the path rule in this checkout. Targets that name no widget — a link
/// to a system, a component, or a `theme::RuiTheme` type — are skipped rather than guessed, and methods
/// whose bodies carry no doc comments at all (`impl` blocks) get `None`, which leaves them unassociated
/// instead of claiming a widget the source never states.
fn builder_widget_from_doc_links(
    m_docs: &str,
    widget_names: &BTreeSet<String>,
) -> (Option<String>, String) {
    let links: Vec<String> = extract_doc_links(&m_docs);
    for link in &links {
        let head: String = link.split("::").next().unwrap_or("").to_lowercase();
        if head.is_empty() {
            continue;
        }
        if widget_names.contains(&head) {
            return (Some(head), "doc_link".to_string());
        }
        // Singular wording in the doc link against a plural declared widget (`window::spawn_window`
        // under a `windows` directory) and the reverse. Only accepted when exactly one declared widget
        // normalizes to this segment, so an ambiguous segment never resolves.
        let norm: String = normalize_widget_term(&head);
        let matches: Vec<String> = widget_names.iter()
            .filter(|w| normalize_widget_term(w) == norm)
            .map(|w| w.to_string())
            .collect();
        if matches.len() == 1usize {
            return (Some(matches[0usize].clone()), "doc_link".to_string());
        }
    }
    (None, String::new())
}

/// Extracts lines `[start_line..=end_line]` (1-based) from raw file source. Also returns the last
/// line actually included so callers can record `code_end_line` honestly when truncation kicked in.
fn slice_lines(source: &str, start_line: usize, end_line: usize, max_lines: usize) -> (String, usize) {
    let lines: Vec<&str> = source.lines().collect();
    if start_line == 0 || start_line > lines.len() {
        return (String::new(), 0usize);
    }
    let start_idx = start_line.saturating_sub(1);
    let end_idx = end_line.min(lines.len()).min(start_idx + max_lines);
    let mut out = lines[start_idx..end_idx].join("\n");
    if end_line > end_idx {
        out.push_str("\n    // ...");
    }
    (out, end_idx)
}

/// Ingests all source files (`src/**/*.rs`), examples (`examples/**/*.rs`), and `README.md` from `rui_dir`.
pub fn ingest_bevy_rui(rui_dir: &Path, version: &str) -> Vec<RuiRecord> {
    let mut records: Vec<RuiRecord> = Vec::new();
    // Map from type name -> default impl source snippet so we can attach `impl Default` / `impl FromWorld` to the struct record
    let mut default_impls: BTreeMap<String, String> = BTreeMap::new();
    // Traits implemented by each type name (`impl Foo { ... }`), stored as a comma-joined list so `implements` can be attached
    let mut impl_traits: BTreeMap<String, String> = BTreeMap::new();

    // Widget names this checkout declares (see `widget_names_in_checkout`). Collected before the file
    // loop so builder methods declared in `src/widgets/mod.rs` can be associated by their documentation
    // instead of left unassociated.
    let widget_names: BTreeSet<String> = widget_names_in_checkout(&rui_dir);

    let src_dir = rui_dir.join("src");
    if src_dir.is_dir() {
        let mut rs_files: Vec<PathBuf> = WalkDir::new(&src_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_file()
                    && e.path().extension().and_then(|s| s.to_str()) == Some("rs")
            })
            .map(|e| e.path().to_path_buf())
            .collect();
        rs_files.sort();

        for file_path in rs_files {
            let Ok(source_text) = fs::read_to_string(&file_path) else {
                continue;
            };
            let Ok(ast) = syn::parse_file(&source_text) else {
                continue;
            };
            let rel_file = file_path
                .strip_prefix(rui_dir)
                .unwrap_or(&file_path)
                .to_string_lossy()
                .replace('\\', "/");

            let (module, widget) = module_and_widget_from_rel(&rel_file);

            // Module-level docs (`//!`)
            let mod_docs = extract_docs(&ast.attrs);
            let mod_name = module
                .rsplit("::")
                .next()
                .unwrap_or("bevy_rui")
                .to_string();
            if !mod_docs.is_empty() || widget.is_some() {
                records.push(RuiRecord {
                    id: format!("module:{module}"),
                    source: "ast".to_string(),
                    kind: "module".to_string(),
                    name: mod_name,
                    full_path: module.clone(),
                    module: module.clone(),
                    widget: widget.clone(),
                    owner: None,
                    signature: format!("pub mod {}", module.rsplit("::").next().unwrap_or("bevy_rui")),
                    doc_links: extract_doc_links(&mod_docs),
                    docs: mod_docs,
                    fields: Vec::new(),
                    derives: Vec::new(),
                    default_impl: None,
                    code: None,
                    code_start_line: 0,
                    code_end_line: 0,
                    widget_from: widget.as_ref().map(|_| "path".to_string()),
                    implements: Vec::new(),
                    example_source: String::new(),
                    file: rel_file.clone(),
                    line: 1,
                    version: version.to_string(),
                });
            }

            for item in &ast.items {
                match item {
                    Item::Struct(s) => {
                        if !is_pub(&s.vis) {
                            continue;
                        }
                        let name = s.ident.to_string();
                        let full_path = format!("{module}::{name}");
                        let docs = extract_docs(&s.attrs);
                        let derives = extract_derives(&s.attrs);
                        let line = s.ident.span().start().line;

                        let mut fields = Vec::new();
                        match &s.fields {
                            Fields::Named(named) => {
                                for f in &named.named {
                                    if let Some(ident) = &f.ident {
                                        fields.push(FieldRecord {
                                            name: ident.to_string(),
                                            ty: clean_tokens(&f.ty),
                                            docs: extract_docs(&f.attrs),
                                        });
                                    }
                                }
                            }
                            Fields::Unnamed(unnamed) => {
                                for (idx, f) in unnamed.unnamed.iter().enumerate() {
                                    fields.push(FieldRecord {
                                        name: idx.to_string(),
                                        ty: clean_tokens(&f.ty),
                                        docs: extract_docs(&f.attrs),
                                    });
                                }
                            }
                            Fields::Unit => {}
                        }

                        let sig = if fields.is_empty() {
                            format!("pub struct {name};")
                        } else {
                            let body = fields
                                .iter()
                                .map(|f| format!("pub {}: {}", f.name, f.ty))
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("pub struct {name} {{ {body} }}")
                        };

                        records.push(RuiRecord {
                            id: format!("struct:{full_path}"),
                            source: "ast".to_string(),
                            kind: "struct".to_string(),
                            name,
                            full_path,
                            module: module.clone(),
                            widget: widget.clone(),
                            owner: None,
                            signature: sig,
                            doc_links: extract_doc_links(&docs),
                            docs,
                            fields,
                            derives,
                            default_impl: None,
                            code: None,
                            code_start_line: 0,
                            code_end_line: 0,
                            widget_from: widget.as_ref().map(|_| "path".to_string()),
                            implements: Vec::new(),
                            example_source: String::new(),
                            file: rel_file.clone(),
                            line,
                            version: version.to_string(),
                        });
                    }

                    Item::Enum(e) => {
                        if !is_pub(&e.vis) {
                            continue;
                        }
                        let name = e.ident.to_string();
                        let full_path = format!("{module}::{name}");
                        let docs = extract_docs(&e.attrs);
                        let derives = extract_derives(&e.attrs);
                        let line = e.ident.span().start().line;

                        let mut variants = Vec::new();
                        for v in &e.variants {
                            let v_name = v.ident.to_string();
                            let v_ty = match &v.fields {
                                Fields::Unit => String::new(),
                                Fields::Unnamed(u) => {
                                    let inner = u
                                        .unnamed
                                        .iter()
                                        .map(|f| clean_tokens(&f.ty))
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    format!("({inner})")
                                }
                                Fields::Named(n) => {
                                    let inner = n
                                        .named
                                        .iter()
                                        .map(|f| {
                                            format!(
                                                "{}: {}",
                                                f.ident
                                                    .as_ref()
                                                    .map(|i| i.to_string())
                                                    .unwrap_or_default(),
                                                clean_tokens(&f.ty)
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    format!("{{ {inner} }}")
                                }
                            };
                            variants.push(FieldRecord {
                                name: v_name,
                                ty: v_ty,
                                docs: extract_docs(&v.attrs),
                            });
                        }

                        let var_list = variants
                            .iter()
                            .map(|v| {
                                if v.ty.is_empty() {
                                    v.name.clone()
                                } else {
                                    format!("{}{}", v.name, v.ty)
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        let sig = format!("pub enum {name} {{ {var_list} }}");

                        records.push(RuiRecord {
                            id: format!("enum:{full_path}"),
                            source: "ast".to_string(),
                            kind: "enum".to_string(),
                            name,
                            full_path,
                            module: module.clone(),
                            widget: widget.clone(),
                            owner: None,
                            signature: sig,
                            doc_links: extract_doc_links(&docs),
                            docs,
                            fields: variants,
                            derives,
                            default_impl: None,
                            code: None,
                            code_start_line: 0,
                            code_end_line: 0,
                            widget_from: widget.as_ref().map(|_| "path".to_string()),
                            implements: Vec::new(),
                            example_source: String::new(),
                            file: rel_file.clone(),
                            line,
                            version: version.to_string(),
                        });
                    }

                    Item::Fn(f) => {
                        if !is_pub(&f.vis) {
                            continue;
                        }
                        let name = f.sig.ident.to_string();
                        let full_path = format!("{module}::{name}");
                        let docs = extract_docs(&f.attrs);
                        let sig = format_fn_signature(&f.sig, true);
                        let start_line = f.sig.ident.span().start().line;
                        let end_line = f.block.brace_token.span.close().end().line;

                        // Capture body snippet for `spawn_*` functions so agents can see exact components and observers
                        let snippet: (String, usize) = if name.starts_with("spawn_") {
                            slice_lines(&source_text, start_line, end_line, 90)
                        } else {
                            (String::new(), 0usize)
                        };
                        let code: Option<String> = if snippet.1 == 0 {
                            None
                        } else {
                            Some(snippet.0)
                        };
                        let code_start_line: usize = if snippet.1 == 0 { 0 } else { start_line };
                        let code_end_line: usize = snippet.1;

                        records.push(RuiRecord {
                            id: format!("fn:{full_path}"),
                            source: "ast".to_string(),
                            kind: "fn".to_string(),
                            name,
                            full_path,
                            module: module.clone(),
                            widget: widget.clone(),
                            owner: None,
                            signature: sig,
                            doc_links: extract_doc_links(&docs),
                            docs,
                            fields: Vec::new(),
                            derives: Vec::new(),
                            default_impl: None,
                            code,
                            code_start_line,
                            code_end_line,
                            widget_from: widget.as_ref().map(|_| "path".to_string()),
                            implements: Vec::new(),
                            example_source: String::new(),
                            file: rel_file.clone(),
                            line: start_line,
                            version: version.to_string(),
                        });
                    }

                    Item::Trait(t) => {
                        if !is_pub(&t.vis) {
                            continue;
                        }
                        let trait_name = t.ident.to_string();
                        let full_path = format!("{module}::{trait_name}");
                        let docs = extract_docs(&t.attrs);
                        let line = t.ident.span().start().line;

                        let mut methods_field = Vec::new();
                        for item in &t.items {
                            if let TraitItem::Fn(m) = item {
                                let m_name = m.sig.ident.to_string();
                                let m_sig = format_fn_signature(&m.sig, false);
                                let m_docs = extract_docs(&m.attrs);
                                let m_line = m.sig.ident.span().start().line;

                                methods_field.push(FieldRecord {
                                    name: m_name.clone(),
                                    ty: m_sig.clone(),
                                    docs: m_docs.clone(),
                                });

                                // Widget association for a trait-method declaration. When the file path
                                // rule already names a widget (`pub trait TooltipExt` in `src/tooltip.rs`),
                                // that is the association and it is recorded as `path`. Otherwise the method
                                // is a builder entry point in a shared file (`fn button` in `src/widgets/mod.rs`),
                                // and its own documentation names the entry point it spawns (``See [`button::spawn_button`]``),
                                // so the widget is taken from that link and recorded as `doc_link`. Methods
                                // whose documentation names no widget keep `None`: no widget is invented from
                                // the method name, which is what stops `fn window` from becoming a `window`
                                // widget when the checkout only declares `windows`.
                                let method_evidence: (Option<String>, String) = builder_widget_from_doc_links(&m_docs, &widget_names);
                                let method_widget: Option<String> = if let Some(w) = &widget {
                                    Some(w.clone())
                                } else {
                                    method_evidence.0
                                };
                                let method_widget_from: Option<String> = if widget.is_some() {
                                    Some("path".to_string())
                                } else if !method_evidence.1.is_empty() {
                                    Some("doc_link".to_string())
                                } else {
                                    None
                                };

                                records.push(RuiRecord {
                                    id: format!("trait_method:{full_path}::{m_name}"),
                                    source: "ast".to_string(),
                                    kind: "trait_method".to_string(),
                                    name: m_name.clone(),
                                    full_path: format!("{full_path}::{m_name}"),
                                    module: module.clone(),
                                    widget: method_widget,
                                    owner: Some(trait_name.clone()),
                                    signature: m_sig,
                                    doc_links: extract_doc_links(&m_docs),
                                    docs: m_docs,
                                    fields: Vec::new(),
                                    derives: Vec::new(),
                                    default_impl: None,
                                    code: None,
                                    code_start_line: 0,
                                    code_end_line: 0,
                                    widget_from: method_widget_from,
                                    implements: Vec::new(),
                                    example_source: String::new(),
                                    file: rel_file.clone(),
                                    line: m_line,
                                    version: version.to_string(),
                                });
                            }
                        }

                        records.push(RuiRecord {
                            id: format!("trait:{full_path}"),
                            source: "ast".to_string(),
                            kind: "trait".to_string(),
                            name: trait_name.clone(),
                            full_path,
                            module: module.clone(),
                            widget: widget.clone(),
                            owner: None,
                            signature: format!("pub trait {trait_name}"),
                            doc_links: extract_doc_links(&docs),
                            docs,
                            fields: methods_field,
                            derives: Vec::new(),
                            default_impl: None,
                            code: None,
                            code_start_line: 0,
                            code_end_line: 0,
                            widget_from: widget.as_ref().map(|_| "path".to_string()),
                            implements: Vec::new(),
                            example_source: String::new(),
                            file: rel_file.clone(),
                            line,
                            version: version.to_string(),
                        });
                    }

                    Item::Impl(imp) => {
                        let self_ty = clean_tokens(&imp.self_ty);
                        let base_owner = self_ty
                            .split('<')
                            .next()
                            .unwrap_or(&self_ty)
                            .rsplit("::")
                            .next()
                            .unwrap_or(&self_ty)
                            .trim()
                            .to_string();

                        if let Some((_, trait_path, _)) = &imp.trait_ {
                            let trait_name = trait_path
                                .segments
                                .last()
                                .map(|s| s.ident.to_string())
                                .unwrap_or_default();
                            if trait_name == "Default" || trait_name == "FromWorld" {
                                let start = imp.brace_token.span.open().start().line;
                                let end = imp.brace_token.span.close().end().line;
                                let (snippet, _) = slice_lines(&source_text, start.saturating_sub(1), end, 60);
                                if !snippet.is_empty() {
                                    default_impls.insert(base_owner.clone(), snippet);
                                }
                            } else if !trait_name.is_empty() {
                                if let Some(existing) = impl_traits.get(&base_owner) {
                                    impl_traits.insert(base_owner.clone(), format!("{}, {}", existing, trait_name.clone()));
                                } else {
                                    impl_traits.insert(base_owner.clone(), trait_name.clone());
                                }
                            }

                            // Trait-impl methods. These are not in scope today at all: a method is only
                            // emitted for inherent `impl Foo { ... }` blocks, so implementations that live
                            // behind a trait (`impl RuiBuilderExt for Commands`, `impl Default for RuiTextBox`,
                            // `impl Plugin for RuiPlugin`) are invisible to the API tools. Capturing them
                            // under the trait they implement is the only honest label available: the same
                            // method name can come from several impl blocks, so the *type* cannot be the
                            // owner without dropping collisions, while the trait is unique per impl block
                            // (`trait_method` records use the trait name the same way). The impl type still
                            // reaches the consumer through `full_path` (module-qualified) and the stored
                            // `code_start_line`/`code_end_line` pointing at the impl block in the file.
                            for impl_item in &imp.items {
                                if let ImplItem::Fn(m) = impl_item {
                                    // No visibility filter here: a trait implementation cannot carry
                                    // `pub` (the trait declaration owns the visibility), so filtering
                                    // on `pub` would discard every implementation body in the checkout.
                                    let m_name = m.sig.ident.to_string();
                                    let m_sig = format_fn_signature(&m.sig, false);
                                    let m_docs = extract_docs(&m.attrs);
                                    let m_line = m.sig.ident.span().start().line;
                                    let start_line = m.sig.ident.span().start().line;
                                    let end_line = m.block.brace_token.span.close().end().line;
                                    // Capturing trait-impl methods is only useful if the consumer can
                                    // still reach them, so every captured method keeps its source span
                                    // and body snippet: an agent asking about a builder entry point or a
                                    // `Default`/`FromWorld` construction path gets the body, not just a name.
                                    let snippet = slice_lines(&source_text, start_line, end_line, 90);
                                    let has_code: bool = snippet.1 > 0usize;
                                    let code: Option<String> = if has_code {
                                        Some(snippet.0.clone())
                                    } else {
                                        None
                                    };
                                    let code_start_line: usize = if has_code { start_line } else { 0usize };
                                    let code_end_line: usize = if has_code { snippet.1 } else { 0usize };
                                    records.push(RuiRecord {
                                        id: format!("trait_impl_method:{module}::{trait_name}::{base_owner}::{m_name}"),
                                        source: "ast".to_string(),
                                        kind: "trait_impl_method".to_string(),
                                        name: m_name.clone(),
                                        full_path: format!("{module}::{trait_name}::{base_owner}::{m_name}"),
                                        module: module.clone(),
                                        // Widget association comes from the file path rule only. The impl
                                        // type is deliberately not turned into a widget name: `impl RuiRootBuilderExt
                                        // for Commands` is not a widget of its own, so guessing from the type name
                                        // would invent an association the source does not state.
                                        widget: widget.clone(),
                                        owner: Some(trait_name.clone()),
                                        signature: m_sig,
                                        doc_links: extract_doc_links(&m_docs),
                                        docs: m_docs,
                                        fields: Vec::new(),
                                        derives: Vec::new(),
                                        default_impl: None,
                                        code,
                                        code_start_line,
                                        code_end_line,
                                        widget_from: widget.as_ref().map(|_| "path".to_string()),
                                        implements: vec![trait_name.clone()],
                                        example_source: String::new(),
                                        file: rel_file.clone(),
                                        line: m_line,
                                        version: version.to_string(),
                                    });
                                }
                            }
                        } else {
                            // Inherent impl methods
                            for impl_item in &imp.items {
                                if let ImplItem::Fn(m) = impl_item {
                                    if !is_pub(&m.vis) {
                                        continue;
                                    }
                                    let m_name = m.sig.ident.to_string();
                                    let m_sig = format_fn_signature(&m.sig, true);
                                    let m_docs = extract_docs(&m.attrs);
                                    let m_line = m.sig.ident.span().start().line;
                                    let full_path = format!("{module}::{base_owner}::{m_name}");

                                    records.push(RuiRecord {
                                        id: format!("method:{full_path}"),
                                        source: "ast".to_string(),
                                        kind: "method".to_string(),
                                        name: m_name,
                                        full_path,
                                        module: module.clone(),
                                        widget: widget.clone(),
                                        owner: Some(base_owner.clone()),
                                        signature: m_sig,
                                        doc_links: extract_doc_links(&m_docs),
                                        docs: m_docs,
                                        fields: Vec::new(),
                                        derives: Vec::new(),
                                        default_impl: None,
                                        code: None,
                                        code_start_line: 0,
                                        code_end_line: 0,
                                        widget_from: widget.as_ref().map(|_| "path".to_string()),
                                        implements: Vec::new(),
                                        example_source: String::new(),
                                        file: rel_file.clone(),
                                        line: m_line,
                                        version: version.to_string(),
                                    });
                                }
                            }
                        }
                    }

                    _ => {}
                }
            }
        }
    }

    // Attach captured `Default`/`FromWorld` implementations and collected implemented traits onto struct/enum/trait records
    for rec in &mut records {
        if rec.kind == "struct" || rec.kind == "enum" || rec.kind == "trait" {
            if let Some(def_code) = default_impls.get(&rec.name) {
                rec.default_impl = Some(def_code.clone());
            }
            if let Some(trait_list) = impl_traits.get(&rec.name) {
                let mut traits: Vec<String> = Vec::new();
                for t in trait_list.split(", ") {
                    let name_t: String = t.to_string();
                    if !traits.contains(&name_t) {
                        traits.push(name_t);
                    }
                }
                rec.implements = traits;
            }
        }
    }

    // Propagate trait participation from owners down to method and trait-method records so the
    // consumer sees `Implements` on a method as a real edge in this schema: a trait method inherits
    // the trait that declares it (`pub trait RuiBuilderExt { fn rui_widget(...); }`), and an inherent
    // impl method inherits whatever `impl ThatTrait for OwnerType` blocks contributed for its owner.
    // Nothing is inferred from names; each edge was observed in an `impl`/`trait` block.
    for rec in &mut records {
        if rec.kind == "trait_method" {
            if let Some(owner) = &rec.owner {
                rec.implements.push(owner.clone());
            }
        }
        if rec.kind == "method" {
            if let Some(owner) = &rec.owner {
                if let Some(trait_list) = impl_traits.get(owner) {
                    for t in trait_list.split(", ") {
                        let name_t: String = t.to_string();
                        if !rec.implements.contains(&name_t) {
                            rec.implements.push(name_t);
                        }
                    }
                }
            }
        }
    }

    // Propagate a builder method's widget association onto the `impl` body that implements it. Trait
    // implementation bodies carry no doc comments in this crate (`impl RuiBuilderExt for Commands` repeats
    // the signature without repeating the `/// See [`button::spawn_button`]` documentation), so they would
    // otherwise be captured as unassociated and `widget_records` could not list them under a widget. The
    // match is not a name equality guess: an impl body inherits an association only when the trait it
    // implements is the very trait that declares a `trait_method` record carrying that widget, which is
    // the relation `impl ... for T { fn button(...) }` states in source. Nothing is read from the impl
    // type, and bodies whose trait declares no widget association stay unassociated.
    // Index the trait declarations that already carry a widget association first (`trait_method` records
    // declared in `src/widgets/mod.rs`), keyed by `owner::name`, so the inheritance pass below never has
    // to borrow `records` while mutating it. The first declaration wins, matching the previous first-match
    // scan order over `records`.
    let decl_widget: BTreeMap<String, (String, String)> = records.iter().fold(
        BTreeMap::new(),
        |mut acc: BTreeMap<String, (String, String)>, decl| {
            if decl.kind == "trait_method" && decl.widget.is_some() {
                let trait_name: String = decl.owner.clone().unwrap_or(String::new()).to_lowercase();
                if !trait_name.is_empty() {
                    let key: String = format!("{trait_name}::{name}", name = decl.name.to_lowercase());
                    if !acc.contains_key(&key) {
                        acc.insert(key, (decl.widget.clone().unwrap(), "decl_trait".to_string()));
                    }
                }
            }
            acc
        },
    );
    for rec in &mut records {
        if rec.kind != "trait_impl_method" || rec.widget.is_some() {
            continue;
        }
        let trait_name: String = rec.owner.clone().unwrap_or(String::new()).to_lowercase();
        if trait_name.is_empty() {
            continue;
        }
        let key: String = format!("{trait_name}::{name}", name = rec.name.to_lowercase());
        if let Some((w, from)) = decl_widget.get(&key) {
            rec.widget = Some(w.clone());
            rec.widget_from = Some(from.clone());
        }
    }

    // Ingest `examples/*.rs`
    let examples_dir = rui_dir.join("examples");
    if examples_dir.is_dir() {
        let mut ex_files: Vec<PathBuf> = WalkDir::new(&examples_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_file()
                    && e.path().extension().and_then(|s| s.to_str()) == Some("rs")
            })
            .map(|e| e.path().to_path_buf())
            .collect();
        ex_files.sort();

        for ex_path in ex_files {
            let Ok(code) = fs::read_to_string(&ex_path) else {
                continue;
            };
            let name = ex_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("example")
                .to_string();
            let rel_file = ex_path
                .strip_prefix(rui_dir)
                .unwrap_or(&ex_path)
                .to_string_lossy()
                .replace('\\', "/");

            // Detect which widgets are used in this example
            let used_widgets = detect_widgets_in_code(&code);
            let code_line_count = code.lines().count();
            let summary = if used_widgets.is_empty() {
                format!("Example `{name}` (`{rel_file}`).")
            } else {
                format!(
                    "Example `{name}` (`{rel_file}`). Uses widgets: {}.",
                    used_widgets.join(", ")
                )
            };

            records.push(RuiRecord {
                id: format!("example:{name}"),
                source: "example".to_string(),
                kind: "example".to_string(),
                name: name.clone(),
                full_path: format!("examples::{name}"),
                module: "examples".to_string(),
                widget: None,
                owner: None,
                signature: format!("cargo run --example {name}"),
                docs: summary,
                fields: used_widgets
                    .into_iter()
                    .map(|w| FieldRecord {
                        name: w,
                        ty: "widget".to_string(),
                        docs: String::new(),
                    })
                    .collect(),
                derives: Vec::new(),
                default_impl: None,
                code: Some(code),
                code_start_line: 1,
                code_end_line: code_line_count,
                doc_links: Vec::new(),
                widget_from: None,
                implements: Vec::new(),
                example_source: name.clone(),
                file: rel_file,
                line: 1,
                version: version.to_string(),
            });
        }
    }

    // Ingest `README.md` if present
    let readme_path = rui_dir.join("README.md");
    if readme_path.is_file() {
        if let Ok(md) = fs::read_to_string(&readme_path) {
            records.push(RuiRecord {
                id: "guide:readme".to_string(),
                source: "markdown".to_string(),
                kind: "guide".to_string(),
                name: "README".to_string(),
                full_path: "bevy_rui::README".to_string(),
                module: "bevy_rui".to_string(),
                widget: None,
                owner: None,
                signature: "README.md".to_string(),
                doc_links: extract_doc_links(&md),
                docs: md,
                fields: Vec::new(),
                derives: Vec::new(),
                default_impl: None,
                code: None,
                code_start_line: 0,
                code_end_line: 0,
                widget_from: None,
                implements: Vec::new(),
                example_source: String::new(),
                file: "README.md".to_string(),
                line: 1,
                version: version.to_string(),
            });
        }
    }

    records
}

/// Scans Rust example code for `RuiBuilderExt` method calls or `spawn_*` invocations to tag used widgets.
fn detect_widgets_in_code(code: &str) -> Vec<String> {
    let widget_patterns: &[(&str, &[&str])] = &[
        ("accordion", &[".accordion(", "spawn_accordion"]),
        ("button", &[".button(", "spawn_button"]),
        ("canvas_layer", &[".canvas_layer(", "spawn_canvas_layer"]),
        ("checkbox", &[".checkbox(", "spawn_checkbox", "RuiCheckbox"]),
        ("code_editor", &[".code_editor(", "spawn_code_editor", "RuiCodeEditor"]),
        ("color_picker", &[".color_picker(", "spawn_color_picker", "RuiColorPicker"]),
        ("context_menu", &[".context_menu(", ".context_menu_item(", "spawn_context_menu"]),
        ("docks", &[".dock_split_horizontal(", ".dock_split_vertical(", ".dock_panel(", "spawn_dock_"]),
        ("dropdown", &[".dropdown(", "spawn_dropdown", "RuiDropdown"]),
        ("file_dialog", &["spawn_file_dialog", "RuiFileDialog", "RuiFileSelected"]),
        ("label", &[".label(", "spawn_label"]),
        ("layout", &[".rui_root(", ".vbox(", ".hbox(", "spawn_vbox", "spawn_hbox"]),
        ("menu", &[".menu_bar(", ".menu_item(", ".submenu(", "spawn_menu_"]),
        ("progress_bar", &[".progress_bar(", "spawn_progress_bar"]),
        ("resizer", &[".resizer(", "spawn_resizer", "RuiResizer"]),
        ("scrollview", &[".scrollview(", "spawn_scrollview", "RuiScrollView"]),
        ("slider", &[".slider(", "spawn_slider", "RuiSlider"]),
        ("tabs", &[".tabs(", "spawn_tabs", "RuiTabsBuilder"]),
        ("textbox", &[".textbox(", ".multiline_textbox(", "spawn_textbox", "RuiTextBox"]),
        ("tooltip", &[".with_tooltip(", "TooltipExt"]),
        ("viewport", &[".viewport(", "spawn_viewport", "RuiViewportCamera"]),
        ("windows", &[".window(", "spawn_window", "RuiWindow"]),
        ("theme", &["RuiTheme", "RuiThemeElement"]),
        ("navigation", &["RuiNavigationConfig", "RuiNavigationBindings", "RuiClickEvent"]),
    ];

    let mut found = Vec::new();
    for (widget, patterns) in widget_patterns {
        if patterns.iter().any(|pat| code.contains(pat)) {
            found.push((*widget).to_string());
        }
    }
    found
}
