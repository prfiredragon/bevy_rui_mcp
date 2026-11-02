//! JSON-RPC 2.0 MCP server over `stdio` for `bevy_rui_mcp`.

use crate::config::ResolvedConfig;
use crate::store::{load_or_build_index, RuiIndex};
use crate::tools::{format_record_detailed, handle_tool_call, tool_definitions};
use crate::tools::{MAX_CODE_BYTES_CAP};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

/// Runs the MCP server over `stdin` / `stdout`.
/// All diagnostic logging goes to `stderr` so `stdout` remains clean JSON-RPC 2.0.
pub fn run_mcp_stdio(mut cfg: ResolvedConfig, force: bool) -> Result<(), String> {
    let mut index: Option<RuiIndex> = match load_or_build_index(&cfg, force) {
        Ok(idx) => {
            eprintln!(
                "[bevy-rui-mcp] loaded index for bevy_rui v{} ({} records, {} widgets)",
                idx.meta.rui_version,
                idx.meta.total_records,
                idx.meta.widgets.len()
            );
            Some(idx)
        }
        Err(err) => {
            eprintln!("[bevy-rui-mcp] warning: {err}");
            None
        }
    };

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let Ok(raw) = line else {
            break;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            let err_resp = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {
                    "code": -32700,
                    "message": "Parse error"
                }
            });
            let _ = writeln!(stdout, "{err_resp}");
            let _ = stdout.flush();
            continue;
        };

        // Notifications have no `id` field
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

        let response = match method {
            "initialize" => {
                let client_proto = params
                    .get("protocolVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("2024-11-05");
                let widget_scope = if let Some(i) = &index {
                    format!("all {} `bevy_rui` widgets", i.meta.widgets.len())
                } else {
                    "every indexed widget (the index could not be loaded at startup)".to_string()
                };
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": client_proto,
                        "capabilities": {
                            "tools": { "listChanged": false },
                            "resources": { "subscribe": false, "listChanged": false }
                        },
                        "serverInfo": {
                            "name": "bevy_rui_mcp",
                            "version": env!("CARGO_PKG_VERSION")
                        },
                        "instructions": format!(
                            "MCP server exclusively for `bevy_rui` (Retained UI widget crate for Bevy 0.20), serving either an index built from a local checkout or a prebuilt index downloaded from GitHub Releases. Use `rui_widget` to inspect {widget_scope}, `rui_api` for exact symbol signatures and `Default` implementations, `rui_search` for hybrid search, `rui_examples` for runnable example code, and `rui_theme` for `RuiTheme` / `RuiThemeElement` styling. `rui_status` reports where the index came from.",
                        )
                    }
                })
            }

            "ping" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),

            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": tool_definitions(index.as_ref())
                }
            }),

            "tools/call" => {
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let (text, is_error) = handle_tool_call(tool_name, &args, &mut cfg, &mut index);
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [
                            {
                                "type": "text",
                                "text": text
                            }
                        ],
                        "isError": is_error
                    }
                })
            }

            "resources/list" => {
                let mut resources = vec![json!({
                    "uri": "rui://widgets",
                    "name": "bevy_rui Widgets Overview",
                    "description": "Overview of all indexed bevy_rui widgets and builder methods.",
                    "mimeType": "text/markdown"
                })];
                if let Some(idx) = &index {
                    for w in &idx.meta.widgets {
                        resources.push(json!({
                            "uri": format!("rui://widget/{w}"),
                            "name": format!("Widget: {w}"),
                            "description": format!("API and spawn functions for the `{w}` widget."),
                            "mimeType": "text/markdown"
                        }));
                    }
                    for ex in idx.records.iter().filter(|r| r.kind == "example") {
                        resources.push(json!({
                            "uri": format!("rui://example/{}", ex.name),
                            "name": format!("Example: {}", ex.name),
                            "description": ex.docs,
                            "mimeType": "text/x-rust"
                        }));
                    }
                }
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "resources": resources
                    }
                })
            }

            "resources/templates/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "resourceTemplates": [
                        {
                            "uriTemplate": "rui://widget/{name}",
                            "name": "bevy_rui Widget Reference",
                            "description": "Documentation, components, and spawn functions for a specific bevy_rui widget.",
                            "mimeType": "text/markdown"
                        },
                        {
                            "uriTemplate": "rui://symbol/{name}",
                            "name": "bevy_rui Symbol Lookup",
                            "description": "Exact symbol documentation for any struct, enum, trait, or fn in bevy_rui.",
                            "mimeType": "text/markdown"
                        },
                        {
                            "uriTemplate": "rui://example/{name}",
                            "name": "bevy_rui Example Source",
                            "description": "Full Rust source code of a bevy_rui example.",
                            "mimeType": "text/x-rust"
                        }
                    ]
                }
            }),

            "resources/read" => {
                let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                if index.is_none() {
                    if let Ok(loaded) = load_or_build_index(&cfg, false) {
                        index = Some(loaded);
                    }
                }
                let text = if uri == "rui://widgets" {
                    let (t, _) = handle_tool_call("rui_widget", &json!({}), &mut cfg, &mut index);
                    t
                } else if let Some(w) = uri.strip_prefix("rui://widget/") {
                    let (t, _) =
                        handle_tool_call("rui_widget", &json!({ "widget": w }), &mut cfg, &mut index);
                    t
                } else if let Some(sym) = uri.strip_prefix("rui://symbol/") {
                    if let Some(idx) = &index {
                        // Resource reads advertise no `max_bytes` argument, so give them the widest
                        // window the tools allow; the header still names any spill-over offset.
                        idx.lookup_symbol(sym)
                            .into_iter()
                            .map(|r| format_record_detailed(r, true, MAX_CODE_BYTES_CAP, 0))
                            .collect::<Vec<_>>()
                            .join("\n---\n")
                    } else {
                        "Index not available.".to_string()
                    }
                } else if let Some(ex) = uri.strip_prefix("rui://example/") {
                    let (t, _) =
                        handle_tool_call("rui_examples", &json!({ "name": ex }), &mut cfg, &mut index);
                    t
                } else {
                    format!("Unknown resource URI `{uri}`.")
                };

                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/markdown",
                                "text": text
                            }
                        ]
                    }
                })
            }

            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {method}")
                }
            }),
        };

        if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
            break;
        }
    }

    Ok(())
}
