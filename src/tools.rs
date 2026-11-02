//! MCP tools implementation and markdown formatting for `bevy_rui_mcp`.

use crate::config::{resolve_config, CliOverrides, ResolvedConfig};
use crate::store::{load_or_build_index, read_registry, version_dir, RuiIndex};
use crate::types::RuiRecord;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Schema fragment for the `kind` filter parameter. `kinds` already includes the `all` sentinel.
fn kind_prop(kinds: Vec<String>) -> Value {
    json!({
        "type": "string",
        "enum": kinds,
        "description": "Optional kind filter (`all` = no filter). Only the kinds listed are indexed.",
    })
}

/// The closed set of record kinds the indexer can produce, plus the `all` sentinel. Used as a fallback
/// when no index is loaded yet (so the schema still describes the valid vocabulary).
fn default_kinds() -> Vec<String> {
    let names: Vec<&str> = vec![
        "all", "enum", "example", "fn", "guide", "method", "module", "struct", "trait", "trait_method",
    ];
    names.iter().map(|name| name.to_string()).collect()
}

/// Schema fragment for the `widget` filter parameter.
/// Enumerates the widget names present in the loaded index; falls back to prose when the index has
/// not been built yet (an empty enum would reject every input). `accept_list` adds the `list` sentinel.
fn widget_prop(widgets: &[String], accept_list: bool) -> Value {
    let mut allowed: Vec<String> = widgets.iter().map(|w| w.clone()).collect();
    if accept_list {
        allowed.push("list".to_string());
    }
    if widgets.is_empty() {
        json!({
            "type": "string",
            "description": "Optional widget name (run `rui_widget` with no arguments to list indexed widgets).",
        })
    } else {
        json!({
            "type": "string",
            "enum": allowed,
            "description": "Widget name; only the names listed are indexed, so any other value matches no record.",
        })
    }
}

/// Returns the JSON Schema definitions for all MCP tools exposed by `bevy_rui_mcp`.
/// `kind`/`widget` filters carry enum constraints: `kind` from the closed set of record kinds the
/// indexer can produce, `widget` from the widget names actually present in the loaded index (when
/// available). This stops agents from inventing filter values that silently match nothing.
pub fn tool_definitions(idx: Option<&RuiIndex>) -> Value {
    // The kinds the indexer can produce. When an index is loaded, take the set actually observed in
    // this index so the enum never advertises a kind that matches no record; `all` stays as the
    // no-filter sentinel either way.
    let kinds: Vec<String> = if let Some(i) = &idx {
        let observed: Vec<String> = i.valid_kinds();
        if observed.is_empty() {
            default_kinds()
        } else {
            let mut with_all: Vec<String> = vec!["all".to_string()];
            with_all.extend(observed);
            with_all
        }
    } else {
        default_kinds()
    };
    let widgets: Vec<String> = if let Some(i) = &idx {
        i.meta.widgets.clone()
    } else {
        Vec::new()
    };

    let widget_count = widgets.len();
    let widget_guide_desc = if widget_count > 0usize {
        format!(
            "Complete reference guide for a `bevy_rui` widget (or list all {} widgets if `widget` is omitted). Returns `RuiBuilderExt` builder methods, `spawn_*` functions, state components, systems, and usage examples.",
            widget_count
        )
    } else {
        "Complete reference guide for a `bevy_rui` widget (lists every indexed widget when `widget` is omitted). Returns `RuiBuilderExt` builder methods, `spawn_*` functions, state components, systems, and usage examples.".to_string()
    };

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
                    "kind": kind_prop(kinds),
                    "widget": widget_prop(&widgets, false),
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
            "description": "Exact symbol lookup for any `bevy_rui` struct, enum, function (`spawn_*`), trait (`RuiBuilderExt`), or method. Returns signature, fields/variants, `Default` implementation, documentation, and spawn code. Mistyped symbols return `Did you mean` suggestions instead of an empty result.",
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
                    },
                    "include_related": {
                        "type": "boolean",
                        "description": "Also return closely related records: the owner type's other methods, siblings in the same module/widget, records that reference this symbol, and records whose docs link to it (default false)."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Max bytes of stored `code` to include (1..=20000, default 4000)."
                    },
                    "code_offset": {
                        "type": "integer",
                        "description": "Byte offset into the record's stored `code` to start at (default 0)."
                    }
                },
                "required": ["symbol"]
            }
        },
        {
            "name": "rui_widget",
            "description": widget_guide_desc,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "widget": widget_prop(&widgets, true)
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
                    "widget": widget_prop(&widgets, false),
                    "max_bytes": {
                        "type": "integer",
                        "description": "Max bytes of example source to return when `name` is given (1..=20000, default 4000)."
                    },
                    "code_offset": {
                        "type": "integer",
                        "description": "Byte offset into the example source to start at (default 0)."
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

pub fn format_record_detailed(
    r: &RuiRecord,
    include_code: bool,
    max_bytes: usize,
    code_offset: usize,
) -> String {
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
        let from = if let Some(f) = &r.widget_from {
            widget_provenance(f.to_string())
        } else {
            "provenance not recorded".to_string()
        };
        out.push_str(&format!("- **Widget:** `{w}` ({from})\n"));
    }
    if !r.implements.is_empty() {
        out.push_str(&format!(
            "- **Implements:** `{}`\n",
            r.implements.join("`, `")
        ));
    }
    if !r.derives.is_empty() {
        out.push_str(&format!("- **Derives:** `{}`\n", r.derives.join(", ")));
    }
    if r.code_start_line > 0 && r.code_end_line > 0 {
        let span = r.code_end_line.saturating_sub(r.code_start_line) + 1;
        let detail = if r.code.is_some() {
            format!("{span} lines stored as `code`")
        } else {
            format!("{span} lines; body not stored")
        };
        out.push_str(&format!(
            "- **Code range:** `{}:{}..{}` ({})\n",
            r.file, r.code_start_line, r.code_end_line, detail
        ));
    }
    if !r.example_source.is_empty() {
        out.push_str(&format!("- **Example source:** `{}.rs`\n", r.example_source));
    }
    if !r.doc_links.is_empty() {
        out.push_str(&format!(
            "- **Doc links ({}):** `{}`\n",
            r.doc_links.len(), r.doc_links.join("`, `")
        ));
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
            out.push_str(&format_code_window(
                "Source Implementation",
                code,
                max_bytes,
                code_offset,
                r.code_start_line,
                r.code_end_line,
            ));
        }
    }

    out
}

/// Default byte cap for stored code when a caller does not pass `max_bytes`.
const DEFAULT_MAX_CODE_BYTES: usize = 4000;
/// Upper bound on `max_bytes` a tool call may ask for, so a long `spawn_*` body still fits one
/// MCP response at the widest setting while smaller windows stay paginable.
pub const MAX_CODE_BYTES_CAP: usize = 20000;

/// Renders one paginated window of a record's stored `code`, always naming the exact slice the
/// reader got plus the byte offset needed for the next window. The indexer never truncates `code`,
/// so pagination is the only place a caller can lose part of a body: reporting the offsets makes
/// that recoverable instead of silently partial. `code_start_line`/`code_end_line` let the reader
/// line up the window with the checkout on disk.
fn format_code_window(
    label: &str,
    code: &str,
    max_bytes: usize,
    code_offset: usize,
    start_line: usize,
    end_line: usize,
) -> String {
    let cap: usize = if max_bytes == 0 { DEFAULT_MAX_CODE_BYTES } else { max_bytes };
    if code_offset >= code.len() {
        return format!(
            "\n**{}:** nothing at `code_offset: {}`; `code` is {} bytes, valid offsets are 0..{}.\n",
            label, code_offset, code.len(), code.len().saturating_sub(1)
        );
    }
    // Never split a UTF-8 sequence: walk the byte cursor over char boundaries.
    let mut end: usize = (code_offset + cap).min(code.len());
    while end > code_offset && !code.is_char_boundary(end) {
        end -= 1;
    }
    let window = &code[code_offset..end];
    let lines = window.lines().count();
    // Report the exact byte window, the total size, and the next offset when a body does not fit
    // in one message. A caller that wants a whole `spawn_*` body walks these windows to completion
    // instead of receiving a silently truncated snippet.
    let head = if end >= code.len() {
        format!(
            "bytes {}..{}/{} of `code`, complete ({} lines)", code_offset, end, code.len(), lines
        )
    } else {
        format!(
            "bytes {}..{}/{} of `code` ({} lines shown)", code_offset, end, code.len(), lines
        )
    };
    let range = if start_line > 0 && end_line > 0 {
        format!(" | source lines `{start_line}..{end_line}`")
    } else {
        "".to_string()
    };
    let more = if end < code.len() {
        format!("\nMore remains: call again with `code_offset: {}`.", end)
    } else {
        " End of `code`.".to_string()
    };
    format!(
        "\n**{}:** {}{}\n```rust\n{}\n```\n{}",
        label, head, range, window, more
    )
}

/// Human-readable provenance for `widget_from`: how the indexer associated a record with a widget,
/// so an agent can tell a widget-tagged record was seen under `src/widgets/` rather than guessed.
fn widget_provenance(from: String) -> String {
    match from.as_str() {
        "path" => "declared under `src/widgets/<widget>.rs`".to_string(),
        "owner" => "owner type belongs to this widget".to_string(),
        "name" => "builder/spawn method named after the widget".to_string(),
        "reexport" => "brought into scope by a `pub use`".to_string(),
        // Emitted by the current indexer for associations derived from source structure rather than
        // the file path, both added when `rui_root`/`vbox`/`window` builders live outside `src/widgets/`.
        "doc_link" => "named by the item's own `/// See [`<widget>::spawn_...`]` doc links".to_string(),
        "decl_trait" => "inherited from the trait that declares this method".to_string(),
        other => other.to_string(),
    }
}

/// Reads the `max_bytes`/`code_offset` code-pagination arguments shared by the tools that return
/// stored source. Defaults to `DEFAULT_MAX_CODE_BYTES` and offset 0; both are clamped so a caller
/// cannot ask for an unbounded window that swamps the MCP response.
fn code_paging_args(args: &Value) -> (usize, usize) {
    let max_bytes: usize = args
        .get("max_bytes")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_MAX_CODE_BYTES)
        .clamp(1, MAX_CODE_BYTES_CAP);
    let code_offset: usize = args
        .get("code_offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(0);
    (max_bytes, code_offset)
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
                let suggestions: Vec<String> = idx.suggest_symbols(query, 8usize);
                if suggestions.is_empty() {
                    return (
                        format!(
                            "No results found in `bevy_rui {}` for query `{query}`.\nAvailable widgets: `{}`",
                            idx.meta.rui_version,
                            idx.meta.widgets.join("`, `")
                        ),
                        false,
                    );
                }
                return (
                    format!(
                        "No results found in `bevy_rui {}` for query `{query}`.\nDid you mean: `{}`\n",
                        idx.meta.rui_version,
                        suggestions.join("`, `")
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
                    "- **`{}`** (`{}`) score `{:.1}` via `{}` — `{}:{}`\n",
                    r.full_path,
                    r.kind,
                    hit.score,
                    hit.reason,
                    r.file,
                    r.line
                ));
                if r.code_start_line > 0 && r.code_end_line > 0 {
                    out.push_str(&format!(
                        "  code `{}`..`{}`\n",
                        r.code_start_line, r.code_end_line
                    ));
                }
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
            let include_related = args
                .get("include_related")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let code_window: (usize, usize) = code_paging_args(args);
            let (max_bytes, code_offset) = code_window;

            let matches = idx.lookup_symbol(symbol);
            if matches.is_empty() {
                let suggestions: Vec<String> = idx.suggest_symbols(symbol, 8usize);
                if suggestions.is_empty() {
                    return (
                        format!(
                            "Symbol `{symbol}` not found in `bevy_rui {}`. Try `rui_search` with a keyword.",
                            idx.meta.rui_version
                        ),
                        false,
                    );
                }
                return (
                    format!(
                        "Symbol `{symbol}` not found in `bevy_rui {}`. Did you mean: `{}`\n",
                        idx.meta.rui_version,
                        suggestions.join("`, `")
                    ),
                    false,
                );
            }

            let mut out = format!(
                "## `bevy_rui {}` API Lookup: `{symbol}`\n\n",
                idx.meta.rui_version
            );
            for rec in &matches {
                out.push_str(&format_record_detailed(rec, include_code, max_bytes, code_offset));
                out.push_str("\n---\n\n");
            }
            if include_related {
                let mut related_lists: Vec<Vec<&RuiRecord>> = Vec::new();
                for rec in &matches {
                    let related = idx.related_records(rec, 8usize);
                    related_lists.push(related);
                }
                out.push_str("### Related Records\n\n");
                let n = matches.len();
                for i in 0..n {
                    if n > 1 {
                        out.push_str(&format!("#### `{}`\n", matches[i].full_path));
                    }
                    for other in &related_lists[i] {
                        out.push_str(&format!("- `{}` (`{}`) — `{}`\n", other.full_path, other.kind, other.signature));
                    }
                    out.push('\n');
                }
                // Backlink pass: records whose doc comments link to each matched symbol. Printed
                // only when there is at least one edge so the section never claims an empty graph.
                let backlink_lists: Vec<Vec<&RuiRecord>> = matches
                    .iter()
                    .map(|rec| idx.referencing_records(rec, 8usize))
                    .collect();
                let has_backlinks: bool = backlink_lists.iter().any(|l| !l.is_empty());
                if has_backlinks {
                    out.push_str("\n### Referenced By\n\n");
                    for i in 0..n {
                        if backlink_lists[i].is_empty() {
                            continue;
                        }
                        // Name each group only when several symbols were asked about, so a
                        // single-symbol lookup stays a flat list.
                        if n > 1 {
                            out.push_str(&format!("#### `{}`\n", matches[i].full_path));
                        }
                        for other in &backlink_lists[i] {
                            out.push_str(
                                &format!("- `{}` (`{}`) — `{}`\n", other.full_path, other.kind, other.signature)
                            );
                        }
                        out.push('\n');
                    }
                }
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
                let suggestions: Vec<String> = idx.suggest_symbols(widget, 8usize);
                let hint = if suggestions.is_empty() {
                    idx.meta.widgets.join("`, `")
                } else {
                    suggestions.join("`, `")
                };
                return (
                    format!(
                        "Unknown widget `{widget}`. Available widgets in `bevy_rui {}`: `{}`",
                        idx.meta.rui_version,
                        hint
                    ),
                    false,
                );
            }

            let mut out = format!(
                "## `bevy_rui {}` Widget Guide: `{widget}`\n\n",
                idx.meta.rui_version
            );
            // A widget guide lists every record in the widget, so code windows use the default cap:
            // the header lines still name the byte range and the next offset when a body spills over.
            for rec in &recs {
                out.push_str(&format_record_detailed(rec, true, DEFAULT_MAX_CODE_BYTES, 0));
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
                    let (max_bytes, code_offset) = code_paging_args(args);
                    let mut out = format!(
                        "## Example `{}` (`{}`)\n\n",
                        ex.name, ex.file
                    );
                    // Name the originating example file and the stored line range before the body so
                    // a reader can find the same example in the checkout.
                    if !ex.example_source.is_empty() {
                        out.push_str(&format!("- **Example source:** `{}.rs`\n", ex.example_source));
                    }
                    if ex.code_start_line > 0 && ex.code_end_line > 0 {
                        out.push_str(&format!(
                            "- **Code range:** `{}:{}`..`{}`\n",
                            ex.file, ex.code_start_line, ex.code_end_line
                        ));
                    }
                    if !ex.docs.is_empty() {
                        out.push_str(&format!("\n{}\n", ex.docs));
                    }
                    if let Some(code) = &ex.code {
                        out.push_str(&format_code_window(
                            "Example Source",
                            code,
                            max_bytes,
                            code_offset,
                            ex.code_start_line,
                            ex.code_end_line,
                        ));
                    } else {
                        out.push_str("\nSource code not stored for this example.\n");
                    }
                    return (out, false);
                } else {
                    let available: Vec<&str> = idx
                        .records
                        .iter()
                        .filter(|r| r.kind == "example")
                        .map(|r| r.name.as_str())
                        .collect();
                    let suggestions: Vec<String> = idx.suggest_symbols(target_name, 6usize);
                    let hint = if suggestions.is_empty() {
                        available.join("`, `")
                    } else {
                        suggestions.join("`, `")
                    };
                    return (
                        format!(
                            "Example `{target_name}` not found. Available examples: `{}`",
                            hint
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
                // Point back at the originating example file and the stored line range so the list
                // is checkable against the checkout, not just a name dump.
                if !ex.example_source.is_empty() {
                    out.push_str(&format!("  - From `{}.rs`\n", ex.example_source));
                }
                if ex.code_start_line > 0 && ex.code_end_line > 0 {
                    out.push_str(&format!("  - Code `{}..{}`\n", ex.code_start_line, ex.code_end_line));
                }
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
                    out.push_str(&format_record_detailed(rec, false, DEFAULT_MAX_CODE_BYTES, 0));
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
                // Graph counters: what the indexer actually extracted this schema. A zero here means
                // ingest produced no such edges, which is distinguishable from "field not surfaced".
                out.push_str(&format!(
                    "- **Doc-link edges:** `{}` (reverse graph over `{}` linked symbols)\n",
                    idx.meta.total_doc_links, idx.linked_by.len()
                ));
                out.push_str(&format!(
                    "- **Implemented-trait edges:** `{}`\n",
                    idx.meta.total_implements
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
