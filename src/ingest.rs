//! Direct Rust AST (`syn`) and example/markdown indexer for `bevy_rui`.
//!
//! Parses `src/**/*.rs`, `examples/**/*.rs`, and `README.md` directly without requiring `cargo doc`.

use crate::types::{FieldRecord, RuiRecord};
use quote::ToTokens;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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

fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
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

/// Extracts lines `[start_line..=end_line]` (1-based) from raw file source.
fn slice_lines(source: &str, start_line: usize, end_line: usize, max_lines: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    if start_line == 0 || start_line > lines.len() {
        return String::new();
    }
    let start_idx = start_line.saturating_sub(1);
    let end_idx = end_line.min(lines.len()).min(start_idx + max_lines);
    let mut out = lines[start_idx..end_idx].join("\n");
    if end_line > end_idx {
        out.push_str("\n    // ...");
    }
    out
}

/// Ingests all source files (`src/**/*.rs`), examples (`examples/**/*.rs`), and `README.md` from `rui_dir`.
pub fn ingest_bevy_rui(rui_dir: &Path, version: &str) -> Vec<RuiRecord> {
    let mut records: Vec<RuiRecord> = Vec::new();
    // Map from type name -> default impl source snippet so we can attach `impl Default` / `impl FromWorld` to the struct record
    let mut default_impls: BTreeMap<String, String> = BTreeMap::new();

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
                    docs: mod_docs,
                    fields: Vec::new(),
                    derives: Vec::new(),
                    default_impl: None,
                    code: None,
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
                            docs,
                            fields,
                            derives,
                            default_impl: None,
                            code: None,
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
                            docs,
                            fields: variants,
                            derives,
                            default_impl: None,
                            code: None,
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
                        let code = if name.starts_with("spawn_") {
                            Some(slice_lines(&source_text, start_line, end_line, 90))
                        } else {
                            None
                        };

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
                            docs,
                            fields: Vec::new(),
                            derives: Vec::new(),
                            default_impl: None,
                            code,
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

                                // Determine if this trait method corresponds to a specific widget
                                let method_widget = widget.clone().or_else(|| {
                                    match m_name.as_str() {
                                        "vbox" | "hbox" | "rui_root" => Some("layout".to_string()),
                                        "multiline_textbox" => Some("textbox".to_string()),
                                        "menu_bar" | "menu_item" | "submenu" => Some("menu".to_string()),
                                        "dock_split_horizontal" | "dock_split_vertical" | "dock_panel" => Some("docks".to_string()),
                                        "context_menu" | "context_menu_item" => Some("context_menu".to_string()),
                                        "with_tooltip" => Some("tooltip".to_string()),
                                        other => Some(other.to_string()),
                                    }
                                });

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
                                    docs: m_docs,
                                    fields: Vec::new(),
                                    derives: Vec::new(),
                                    default_impl: None,
                                    code: None,
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
                            docs,
                            fields: methods_field,
                            derives: Vec::new(),
                            default_impl: None,
                            code: None,
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
                                let snippet = slice_lines(&source_text, start.saturating_sub(1), end, 60);
                                if !snippet.is_empty() {
                                    default_impls.insert(base_owner, snippet);
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
                                        docs: m_docs,
                                        fields: Vec::new(),
                                        derives: Vec::new(),
                                        default_impl: None,
                                        code: None,
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

    // Attach captured `Default` / `FromWorld` implementations onto their corresponding struct/enum records
    for rec in &mut records {
        if rec.kind == "struct" || rec.kind == "enum" {
            if let Some(def_code) = default_impls.get(&rec.name) {
                rec.default_impl = Some(def_code.clone());
            }
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
                docs: md,
                fields: Vec::new(),
                derives: Vec::new(),
                default_impl: None,
                code: None,
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
