//! MCP tools implementation and markdown formatting for `bevy_rui_mcp`.

use crate::config::{resolve_config, CliOverrides, ResolvedConfig};
use crate::store::{load_or_build_index, read_registry, version_dir, RuiIndex};
use crate::types::RuiRecord;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Returns the JSON Schema definitions for all MCP tools exposed by `bevy_rui_mcp`.
pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "rui_search",
            "description": "Hybrid search across `bevy_rui` widgets, API symbols (`struct`, `enum`, `fn`, `trait_method`), theme elements, and examples.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query (symbol name like `spawn_button` or concept like `split dock panel` or `file dialog filter`)."
                    },
                    "kind": {
                        "type": "string",
                        "description": "Optional kind filter: `all` (default), `struct`, `enum`, `fn`, `trait`, `trait_method`, `method`, `module`, `example`."
                    },
                    "widget": {
                        "type": "string",
                        "description": "Optional widget filter (e.g. `button`, `docks`, `code_editor`, `file_dialog`, `tabs`, `menu`, `windows`)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results to return (1..=25, default 10)."
                    }
                },
                "required": ["query"]
            }
        },
        {
            "name": "rui_api",
            "description": "Exact symbol lookup for any `bevy_rui` struct, enum, function (`spawn_*`), trait (`RuiBuilderExt`), or method. Returns signature, fields/variants, `Default` implementation, documentation, and spawn code.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Symbol name or path (e.g. `spawn_button`, `RuiTheme`, `RuiBuilderExt::dock_split_horizontal`, `RuiCodeEditor`, `RuiFileDialog`)."
                    },
                    "include_code": {
                        "type": "boolean",
                        "description": "Include full `spawn_*` implementation body if available (default true)."
                    }
                },
                "required": ["symbol"]
            }
        },
        {
            "name": "rui_widget",
            "description": "Complete reference guide for a `bevy_rui` widget (or list of all 21 widgets if `widget` is omitted). Returns `RuiBuilderExt` builder methods, `spawn_*` functions, state components, systems, and usage examples.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "widget": {
                        "type": "string",
                        "description": "Widget name (e.g. `accordion`, `button`, `canvas_layer`, `checkbox`, `clipboard`, `code_editor`, `color_picker`, `context_menu`, `docks`, `dropdown`, `file_dialog`, `label`, `layout`, `menu`, `progress_bar`, `resizer`, `scrollview`, `slider`, `tabs`, `textbox`, `tooltip`, `viewport`, `windows`). Omit or pass `list` to list all widgets."
                    }
                }
            }
        },
        {
            "name": "rui_examples",
            "description": "List `bevy_rui` example programs or retrieve the full source code of a specific example (or examples using a specific widget).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Exact example name (e.g. `layout_dock`, `layout_dock_save`, `layout_dock_scroll`, `layout_test`, `game_test`, `code_editor_demo`, `file_dialog_test`, `platformer_hud`)."
                    },
                    "widget": {
                        "type": "string",
                        "description": "Filter examples that demonstrate a specific widget (e.g. `docks`, `code_editor`, `file_dialog`, `menu`)."
                    }
                }
            }
        },
        {
            "name": "rui_theme",
            "description": "Inspect `bevy_rui`'s theming system (`RuiTheme`, `RuiThemeElement`, 9-slice textures, colors, and borders). Optionally filter fields/variants by keyword.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "description": "Optional keyword to filter `RuiTheme` fields and `RuiThemeElement` variants (e.g. `button`, `window`, `slider`, `border`, `image`)."
                    }
                }
            }
        },
        {
            "name": "rui_status",
            "description": "Show `bevy_rui_mcp` index status, resolved paths (`rui_path`, `data_dir`), indexed version, widget count, and optionally re-index from a local `bevy_rui` path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reindex": {
                        "type": "boolean",
                        "description": "Set to true to force rebuilding the index from the local `bevy_rui` directory."
                    },
                    "rui_path": {
                        "type": "string",
                        "description": "Optional custom path to a `bevy_rui` directory to index on the fly."
                    }
                }
            }
        }
    ])
}

pub fn format_record_detailed(r: &RuiRecord, include_code: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "### `{}` ({})\n",
        r.full_path, r.kind
    ));
    out.push_str(&format!(
        "- **File:** `{}:{}` | **Version:** `bevy_rui {}`\n",
        r.file, r.line, r.version
    ));
    if let Some(w) = &r.widget {
        out.push_str(&format!("- **Widget:** `{w}`\n"));
    }
    if !r.derives.is_empty() {
        out.push_str(&format!("- **Derives:** `{}`\n", r.derives.join(", ")));
    }
    out.push_str(&format!("\n```rust\n{}\n```\n", r.signature));

    if !r.docs.is_empty() {
        out.push_str(&format!("\n{}\n", r.docs));
    }

    if !r.fields.is_empty() {
        let label = match r.kind.as_str() {
            "enum" => "Variants",
            "trait" => "Methods",
            "example" => "Widgets Used",
            _ => "Fields",
        };
        out.push_str(&format!("\n**{label}:**\n"));
        for f in &r.fields {
            if f.ty.is_empty() || f.ty == "widget" {
                if f.docs.is_empty() {
                    out.push_str(&format!("- `{}`\n", f.name));
                } else {
                    out.push_str(&format!("- `{}` — {}\n", f.name, f.docs));
                }
            } else if f.docs.is_empty() {
                out.push_str(&format!("- `{}: {}`\n", f.name, f.ty));
            } else {
                out.push_str(&format!("- `{}: {}` — {}\n", f.name, f.ty, f.docs));
            }
        }
    }

    if let Some(def) = &r.default_impl {
        out.push_str(&format!(
            "\n**Default / FromWorld Implementation:**\n```rust\n{def}\n```\n"
        ));
    }

    if include_code {
        if let Some(code) = &r.code {
            out.push_str(&format!("\n**Source Implementation:**\n```rust\n{code}\n```\n"));
        }
    }

    out
}

/// Dispatches a tool call and returns `(markdown_text, is_error)`.
pub fn handle_tool_call(
    name: &str,
    args: &Value,
    cfg: &mut ResolvedConfig,
    index: &mut Option<RuiIndex>,
) -> (String, bool) {
    // Ensure index is loaded (unless calling rui_status which handles its own loading/reindexing)
    if name != "rui_status" && index.is_none() {
        match load_or_build_index(cfg, false) {
            Ok(loaded) => *index = Some(loaded),
            Err(e) => return (e, true),
        }
    }

    match name {
        "rui_search" => {
            let Some(idx) = index.as_ref() else {
                return ("Index not loaded.".to_string(), true);
            };
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if query.is_empty() {
                return ("Parameter `query` is required.".to_string(), true);
            }
            let kind = args.get("kind").and_then(|v| v.as_str());
            let widget = args.get("widget").and_then(|v| v.as_str());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(cfg.max_results)
                .clamp(1, 25);

            let hits = idx.search(query, kind, widget, limit);
            if hits.is_empty() {
                return (
                    format!(
                        "No results found in `bevy_rui {}` for query `{query}`.\nAvailable widgets: `{}`",
                        idx.meta.rui_version,
                        idx.meta.widgets.join("`, `")
                    ),
                    false,
                );
            }

            let mut out = format!(
                "## `bevy_rui {}` Search Results for `{query}` ({} hits)\n\n",
                idx.meta.rui_version,
                hits.len()
            );
            for hit in hits {
                let r = hit.record;
                out.push_str(&format!(
                    "- **`{}`** (`{}`) — `{}:{}`\n",
                    r.full_path, r.kind, r.file, r.line
                ));
                out.push_str(&format!("  ```rust\n  {}\n  ```\n", r.signature));
                if !r.docs.is_empty() {
                    let first_para = r.docs.lines().take(3).collect::<Vec<_>>().join(" ");
                    out.push_str(&format!("  {}\n", first_para));
                }
                out.push('\n');
            }
            (out, false)
        }

        "rui_api" => {
            let Some(idx) = index.as_ref() else {
                return ("Index not loaded.".to_string(), true);
            };
            let symbol = args
                .get("symbol")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if symbol.is_empty() {
                return ("Parameter `symbol` is required.".to_string(), true);
            }
            let include_code = args
                .get("include_code")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let matches = idx.lookup_symbol(symbol);
            if matches.is_empty() {
                return (
                    format!(
                        "Symbol `{symbol}` not found in `bevy_rui {}`. Try `rui_search` with a keyword.",
                        idx.meta.rui_version
                    ),
                    false,
                );
            }

            let mut out = format!(
                "## `bevy_rui {}` API Lookup: `{symbol}`\n\n",
                idx.meta.rui_version
            );
            for rec in matches {
                out.push_str(&format_record_detailed(rec, include_code));
                out.push_str("\n---\n\n");
            }
            (out, false)
        }

        "rui_widget" => {
            let Some(idx) = index.as_ref() else {
                return ("Index not loaded.".to_string(), true);
            };
            let widget = args
                .get("widget")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();

            if widget.is_empty() || widget.eq_ignore_ascii_case("list") {
                let mut out = format!(
                    "## `bevy_rui {}` Widgets ({})\n\n",
                    idx.meta.rui_version,
                    idx.meta.widgets.len()
                );
                for w in &idx.meta.widgets {
                    let recs = idx.widget_records(w);
                    let mod_doc = recs
                        .iter()
                        .find(|r| r.kind == "module")
                        .map(|r| r.docs.lines().next().unwrap_or("").to_string())
                        .unwrap_or_default();
                    let builders: Vec<String> = recs
                        .iter()
                        .filter(|r| r.kind == "trait_method" || (r.kind == "fn" && r.name.starts_with("spawn_")))
                        .map(|r| format!("`{}`", r.name))
                        .collect();
                    out.push_str(&format!(
                        "- **`{w}`**: {}\n  - Entry points: {}\n",
                        mod_doc,
                        builders.join(", ")
                    ));
                }
                return (out, false);
            }

            let recs = idx.widget_records(widget);
            if recs.is_empty() {
                return (
                    format!(
                        "Unknown widget `{widget}`. Available widgets in `bevy_rui {}`: `{}`",
                        idx.meta.rui_version,
                        idx.meta.widgets.join("`, `")
                    ),
                    false,
                );
            }

            let mut out = format!(
                "## `bevy_rui {}` Widget Guide: `{widget}`\n\n",
                idx.meta.rui_version
            );
            for rec in &recs {
                out.push_str(&format_record_detailed(rec, true));
                out.push('\n');
            }

            let examples = idx.examples_for_widget(Some(widget));
            if !examples.is_empty() {
                out.push_str("### Examples Using This Widget\n");
                for ex in examples {
                    out.push_str(&format!(
                        "- **`{}`** (`{}`) — run `rui_examples` with `name: \"{}\"` to view source.\n",
                        ex.name, ex.file, ex.name
                    ));
                }
            }
            (out, false)
        }

        "rui_examples" => {
            let Some(idx) = index.as_ref() else {
                return ("Index not loaded.".to_string(), true);
            };
            let ex_name = args
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let widget = args
                .get("widget")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            if let Some(target_name) = ex_name {
                let clean = target_name.strip_suffix(".rs").unwrap_or(target_name);
                if let Some(ex) = idx
                    .records
                    .iter()
                    .find(|r| r.kind == "example" && r.name.eq_ignore_ascii_case(clean))
                {
                    let mut out = format!(
                        "## Example `{}` (`{}`)\n\n{}\n\n",
                        ex.name, ex.file, ex.docs
                    );
                    if let Some(code) = &ex.code {
                        out.push_str(&format!("```rust\n{code}\n```\n"));
                    }
                    return (out, false);
                } else {
                    let available: Vec<&str> = idx
                        .records
                        .iter()
                        .filter(|r| r.kind == "example")
                        .map(|r| r.name.as_str())
                        .collect();
                    return (
                        format!(
                            "Example `{target_name}` not found. Available examples: `{}`",
                            available.join("`, `")
                        ),
                        false,
                    );
                }
            }

            let examples = idx.examples_for_widget(widget);
            let mut out = format!(
                "## `bevy_rui {}` Examples ({})\n\n",
                idx.meta.rui_version,
                examples.len()
            );
            for ex in examples {
                let widgets: Vec<&str> = ex.fields.iter().map(|f| f.name.as_str()).collect();
                out.push_str(&format!(
                    "- **`{}`** (`{}`)\n  - Widgets used: `{}`\n  - Run `{}`\n",
                    ex.name,
                    ex.file,
                    widgets.join("`, `"),
                    ex.signature
                ));
            }
            out.push_str("\nPass `name: \"<example_name>\"` to `rui_examples` to view the full Rust source code.");
            (out, false)
        }

        "rui_theme" => {
            let Some(idx) = index.as_ref() else {
                return ("Index not loaded.".to_string(), true);
            };
            let filter = args
                .get("filter")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty());

            let mut out = format!(
                "## `bevy_rui {}` Theme Reference (`RuiTheme` & `RuiThemeElement`)\n\n",
                idx.meta.rui_version
            );

            for rec in idx.records.iter().filter(|r| r.module == "bevy_rui::theme") {
                if let Some(flt) = &filter {
                    if rec.kind == "struct" || rec.kind == "enum" {
                        out.push_str(&format!("### `{}` (`{}`)\n", rec.full_path, rec.kind));
                        for f in &rec.fields {
                            let hay = format!("{} {} {}", f.name, f.ty, f.docs).to_ascii_lowercase();
                            if hay.contains(flt) {
                                if f.ty.is_empty() {
                                    out.push_str(&format!("- `{}` — {}\n", f.name, f.docs));
                                } else {
                                    out.push_str(&format!("- `{}: {}` {}\n", f.name, f.ty, f.docs));
                                }
                            }
                        }
                        out.push('\n');
                    }
                } else {
                    out.push_str(&format_record_detailed(rec, false));
                    out.push('\n');
                }
            }
            (out, false)
        }

        "rui_status" => {
            let force = args
                .get("reindex")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let custom_path = args
                .get("rui_path")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());

            if let Some(p) = custom_path {
                let overrides = CliOverrides {
                    rui_path: Some(PathBuf::from(p)),
                    project_root: Some(cfg.project_root.clone()),
                    data_dir: Some(cfg.data_dir.clone()),
                    version: None,
                    repo: Some(cfg.repo.clone()),
                    force: true,
                };
                *cfg = resolve_config(&overrides);
            }

            if force || custom_path.is_some() || index.is_none() {
                match load_or_build_index(cfg, force || custom_path.is_some()) {
                    Ok(rebuilt) => *index = Some(rebuilt),
                    Err(e) => return (e, true),
                }
            }

            let reg = read_registry(&cfg.data_dir);
            let installed_versions: Vec<String> = reg.versions.keys().cloned().collect();
            let v_dir = version_dir(&cfg.data_dir, &cfg.rui_version);

            let mut out = String::from("## `bevy_rui_mcp` Status\n\n");
            out.push_str(&format!(
                "- **`bevy_rui` Version:** `{}` ({})\n",
                cfg.rui_version, cfg.version_source
            ));
            if let Some(bv) = &cfg.bevy_version {
                out.push_str(&format!("- **Target Bevy Version:** `{bv}`\n"));
            }
            out.push_str(&format!(
                "- **Local `bevy_rui` Path:** `{}` ({})\n",
                cfg.rui_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "none (using persisted index)".to_string()),
                cfg.rui_path_source
            ));
            out.push_str(&format!(
                "- **Data Directory:** `{}`\n",
                cfg.data_dir.display()
            ));
            out.push_str(&format!(
                "- **Active Index Cache:** `{}`\n",
                v_dir.display()
            ));
            out.push_str(&format!(
                "- **Installed Versions in Registry:** `{}`\n",
                if installed_versions.is_empty() {
                    "none".to_string()
                } else {
                    installed_versions.join("`, `")
                }
            ));

            if let Some(idx) = index.as_ref() {
                out.push_str(&format!(
                    "- **Indexed Records:** `{}` (`{}` unique symbol keys)\n",
                    idx.meta.total_records, idx.meta.total_symbols
                ));
                out.push_str(&format!(
                    "- **Widgets ({}):** `{}`\n",
                    idx.meta.widgets.len(),
                    idx.meta.widgets.join("`, `")
                ));
                out.push_str("- **Breakdown by Kind:**\n");
                for (k, count) in &idx.meta.by_kind {
                    out.push_str(&format!("  - `{k}`: {count}\n"));
                }
            }
            (out, false)
        }

        other => (format!("Unknown tool `{other}`."), true),
    }
}
