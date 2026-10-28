//! `bevy-rui-mcp` — Native Rust MCP server and indexer for `bevy_rui`.
//!
//! Supports indexing any local `bevy_rui` directory (`--rui-path` or `BEVY_RUI_PATH`)
//! as well as building, publishing, and fetching prebuilt `.tar.gz` index bundles from GitHub Releases.

pub mod config;
pub mod ingest;
pub mod mcp;
pub mod store;
pub mod tools;
pub mod types;

use config::{expand_home, resolve_config, CliOverrides};
use store::{
    fetch_index_bundle, load_or_build_index, publish_index_bundle, read_registry,
    save_index_to_disk, version_dir,
};
use std::env;
use std::path::PathBuf;
use std::process;

const USAGE: &str = "\
bevy-rui-mcp — MCP server and AST/example indexer exclusively for bevy_rui

USAGE:
  bevy-rui-mcp [serve] [OPTIONS]             Start the MCP server over stdio (default)
  bevy-rui-mcp build-index [ver] [OPTIONS]   Index a local bevy_rui directory and save to data_dir
  bevy-rui-mcp publish-index [ver] [OPTIONS] Package a built index into dist/bevy-rui-index-<ver>.tar.gz + index-manifest.json
  bevy-rui-mcp fetch-index [ver] [OPTIONS]   Download and install a prebuilt index from GitHub Releases (or --from <url/file>)
  bevy-rui-mcp status [OPTIONS]              Print resolved paths, version, and index status
  bevy-rui-mcp help                          Show this help message

OPTIONS:
  --rui-path <dir>       Path to the local `bevy_rui` crate directory (env: BEVY_RUI_PATH)
  --project-root <dir>   Path to your game/app Cargo project root (env: BEVY_RUI_PROJECT_ROOT)
  --data-dir <dir>       Directory to store/read persisted indexes (env: BEVY_RUI_DATA_DIR)
  --version <ver>        Explicit `bevy_rui` version (env: BEVY_RUI_VERSION)
  --repo <owner/repo>    GitHub repository for `fetch-index` (env: BEVY_RUI_REPO, default: prfiredragon/bevy_rui_mcp)
  --from <url-or-file>   Explicit `.tar.gz` URL or local file path for `fetch-index`
  --out <dir>            Output directory for `publish-index` (default: ./dist) or `data_dir` override for `fetch-index`
  --revision <num>       Numeric bundle revision for `publish-index` (default: 0)
  --force                Force rebuilding or re-downloading the index
";

fn parse_args() -> (String, CliOverrides, Option<String>, Option<PathBuf>, u32) {
    let mut args = env::args().skip(1).peekable();
    let mut subcommand = "serve".to_string();
    let mut overrides = CliOverrides::default();
    let mut from_source: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut revision: u32 = 0;

    if let Some(first) = args.peek() {
        if !first.starts_with('-') {
            let cmd = args.next().unwrap();
            match cmd.as_str() {
                "serve" | "start" | "mcp" | "build-index" | "reindex" | "publish-index"
                | "fetch-index" | "status" | "help" => {
                    subcommand = cmd;
                }
                ver => {
                    overrides.version = Some(ver.to_string());
                }
            }
        }
    }

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => subcommand = "help".to_string(),
            "--force" | "-f" => overrides.force = true,
            "--rui-path" => {
                if let Some(v) = args.next() {
                    overrides.rui_path = Some(expand_home(&v));
                }
            }
            "--project-root" => {
                if let Some(v) = args.next() {
                    overrides.project_root = Some(expand_home(&v));
                }
            }
            "--data-dir" | "--data" => {
                if let Some(v) = args.next() {
                    overrides.data_dir = Some(expand_home(&v));
                }
            }
            "--version" | "-v" => {
                if let Some(v) = args.next() {
                    overrides.version = Some(v);
                }
            }
            "--repo" => {
                if let Some(v) = args.next() {
                    overrides.repo = Some(v);
                }
            }
            "--from" => {
                if let Some(v) = args.next() {
                    from_source = Some(v);
                }
            }
            "--out" => {
                if let Some(v) = args.next() {
                    out_dir = Some(expand_home(&v));
                }
            }
            "--revision" => {
                if let Some(v) = args.next() {
                    revision = v.parse::<u32>().unwrap_or(0);
                }
            }
            other if !other.starts_with('-') && overrides.version.is_none() => {
                overrides.version = Some(other.to_string());
            }
            _ => {}
        }
    }

    (subcommand, overrides, from_source, out_dir, revision)
}

fn main() {
    let (subcommand, mut overrides, from_source, out_dir, revision) = parse_args();

    if subcommand == "help" {
        print!("{USAGE}");
        return;
    }

    if subcommand == "fetch-index" {
        if let Some(out) = &out_dir {
            overrides.data_dir = Some(out.clone());
        }
    }

    let cfg = resolve_config(&overrides);

    match subcommand.as_str() {
        "serve" | "start" | "mcp" => {
            if let Err(e) = mcp::run_mcp_stdio(cfg, overrides.force) {
                eprintln!("[bevy-rui-mcp] error: {e}");
                process::exit(1);
            }
        }

        "build-index" | "reindex" => {
            let force = overrides.force || subcommand == "reindex";
            match load_or_build_index(&cfg, force) {
                Ok(index) => match save_index_to_disk(&cfg.data_dir, &index) {
                    Ok(dir) => {
                        println!(
                            "Built index for bevy_rui v{} ({} records, {} widgets, {} symbol keys)",
                            index.meta.rui_version,
                            index.meta.total_records,
                            index.meta.widgets.len(),
                            index.meta.total_symbols
                        );
                        println!("  source   : {}", cfg.rui_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "cached".to_string()));
                        println!("  saved to : {}", dir.display());
                    }
                    Err(e) => {
                        eprintln!("[bevy-rui-mcp] failed to save index: {e}");
                        process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("[bevy-rui-mcp] build-index failed: {e}");
                    process::exit(1);
                }
            }
        }

        "publish-index" => {
            let target_out = out_dir.unwrap_or_else(|| PathBuf::from("dist"));
            match publish_index_bundle(&cfg, &target_out, revision) {
                Ok((bundle, manifest)) => {
                    println!("Packaged prebuilt index bundle for bevy_rui v{}:", cfg.rui_version);
                    println!("  archive  : {}", bundle.display());
                    println!("  manifest : {}", manifest.display());
                    println!(
                        "\nUpload to GitHub Releases:\n  gh release create v{0} {1} {2} --title \"bevy_rui v{0} index\"",
                        cfg.rui_version,
                        bundle.display(),
                        manifest.display()
                    );
                }
                Err(e) => {
                    eprintln!("[bevy-rui-mcp] publish-index failed: {e}");
                    process::exit(1);
                }
            }
        }

        "fetch-index" => {
            println!(
                "bevy-rui-mcp fetch-index — bevy_rui v{}\n  data dir : {}",
                cfg.rui_version,
                cfg.data_dir.display()
            );
            match fetch_index_bundle(&cfg, from_source.as_deref(), overrides.force) {
                Ok(dir) => {
                    println!("Installed prebuilt index at {}", dir.display());
                }
                Err(e) => {
                    eprintln!("[bevy-rui-mcp] fetch-index failed: {e}");
                    process::exit(1);
                }
            }
        }

        "status" => {
            let v_dir = version_dir(&cfg.data_dir, &cfg.rui_version);
            let has_persisted = v_dir.join("records.ndjson").is_file();
            let reg = read_registry(&cfg.data_dir);
            println!("bevy-rui-mcp status");
            println!("  project root   : {}", cfg.project_root.display());
            println!(
                "  rui_path       : {} ({})",
                cfg.rui_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(none)".to_string()),
                cfg.rui_path_source
            );
            println!(
                "  rui_version    : {} ({})",
                cfg.rui_version, cfg.version_source
            );
            if let Some(bv) = &cfg.bevy_version {
                println!("  bevy_version   : {bv}");
            }
            println!("  data_dir       : {}", cfg.data_dir.display());
            println!("  github repo    : {}", cfg.repo);
            println!(
                "  index on disk  : {}",
                if has_persisted {
                    format!("yes ({})", v_dir.display())
                } else {
                    "no (will build on demand from rui_path or run `bevy-rui-mcp fetch-index`)".to_string()
                }
            );
            if !reg.versions.is_empty() {
                let list: Vec<&str> = reg.versions.keys().map(|s| s.as_str()).collect();
                println!("  installed vers : {}", list.join(", "));
            }
        }

        other => {
            eprintln!("[bevy-rui-mcp] unknown command `{other}`\n\n{USAGE}");
            process::exit(1);
        }
    }
}
