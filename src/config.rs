//! Flexible configuration and path resolution for `bevy_rui_mcp`.
//!
//! Resolves `rui_path`, `project_root`, `data_dir`, and `version` from:
//! 1. CLI flags (`--rui-path`, `--project-root`, `--data-dir`, `--version`, `--repo`)
//! 2. Environment variables (`BEVY_RUI_PATH`, `BEVY_RUI_PROJECT_ROOT`, `BEVY_RUI_DATA_DIR`, `BEVY_RUI_VERSION`, `BEVY_RUI_REPO`)
//! 3. Config file (`bevy-rui-mcp.config.json` in project root, executable directory, or `~/.config/bevy-rui-mcp/config.json`)
//! 4. Automatic Cargo workspace / dependency discovery (`Cargo.toml` / `Cargo.lock` path or version dependencies, or sibling `../bevy_rui` directories).
//! 5. An installed prebuilt index under `<data_dir>/versions` (a source-free setup — one that only
//!    downloaded a bundle from GitHub Releases — needs the `bevy_rui` package *name* and *version*,
//!    never a source tree, a config file, or a location hardcoded for one developer's machine).

use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use crate::store::{read_registry, version_dir};

#[derive(Debug, Default, Clone)]
pub struct CliOverrides {
    pub rui_path: Option<PathBuf>,
    pub project_root: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub version: Option<String>,
    pub repo: Option<String>,
    pub force: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFile {
    rui_path: Option<String>,
    project_root: Option<String>,
    data_dir: Option<String>,
    version: Option<String>,
    repo: Option<String>,
    max_results: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Working Cargo project root (if any).
    pub project_root: PathBuf,
    /// Resolved path to the `bevy_rui` source checkout (if available locally).
    pub rui_path: Option<PathBuf>,
    /// How `rui_path` was found.
    pub rui_path_source: String,
    /// Resolved `bevy_rui` version (e.g. `"0.1.7"`).
    pub rui_version: String,
    /// How `rui_version` was detected.
    pub version_source: String,
    /// Target Bevy version declared by `bevy_rui` (e.g. `"0.20.0"`).
    pub bevy_version: Option<String>,
    /// Directory where persisted indexes (`versions/<ver>/`) are stored.
    pub data_dir: PathBuf,
    /// GitHub repository `owner/name` used by `fetch-index`.
    pub repo: String,
    /// Default max search results.
    pub max_results: usize,
    /// Config file path if one was loaded.
    pub config_file: Option<PathBuf>,
}

pub fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
        if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
            if raw == "~" {
                return PathBuf::from(home);
            }
            return PathBuf::from(home).join(&raw[2..]);
        }
    }
    PathBuf::from(raw)
}

fn resolve_rel_path(raw: &str, base: &Path) -> PathBuf {
    let expanded = expand_home(raw);
    if expanded.is_absolute() {
        expanded
    } else {
        base.join(expanded)
    }
}

/// Checks if `dir` looks like a valid `bevy_rui` crate root (has `Cargo.toml` with `name = "bevy_rui"` or `src/widgets`).
pub fn is_bevy_rui_dir(dir: &Path) -> bool {
    let cargo_toml = dir.join("Cargo.toml");
    if cargo_toml.is_file() {
        if let Ok(text) = fs::read_to_string(&cargo_toml) {
            if text.contains("name = \"bevy_rui\"") || text.contains("name=\"bevy_rui\"") {
                return true;
            }
        }
    }
    dir.join("src").join("widgets").is_dir() && dir.join("src").join("lib.rs").is_file()
}

/// Reads `version` and `bevy` dependency version from a `bevy_rui/Cargo.toml`.
pub fn read_rui_manifest_info(rui_dir: &Path) -> (Option<String>, Option<String>) {
    let toml_path = rui_dir.join("Cargo.toml");
    let Ok(text) = fs::read_to_string(toml_path) else {
        return (None, None);
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return (None, None);
    };
    let pkg_version = value
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let bevy_version = value
        .get("dependencies")
        .and_then(|d| d.get("bevy"))
        .and_then(|b| {
            if let Some(s) = b.as_str() {
                Some(s.to_string())
            } else {
                b.get("version").and_then(|v| v.as_str()).map(|s| s.to_string())
            }
        });

    (pkg_version, bevy_version)
}

/// Searches `Cargo.toml` and `Cargo.lock` upward from `start_dir` for a `bevy_rui` dependency
/// (returning `(Option<path_dep>, Option<version>, source_desc)`).
fn detect_from_cargo_project(start_dir: &Path) -> (Option<PathBuf>, Option<String>, Option<String>) {
    let mut cur = Some(start_dir);
    while let Some(dir) = cur {
        let toml_path = dir.join("Cargo.toml");
        if toml_path.is_file() {
            if let Ok(text) = fs::read_to_string(&toml_path) {
                if let Ok(val) = text.parse::<toml::Value>() {
                    // 1. Is this Cargo.toml itself bevy_rui?
                    if val
                        .get("package")
                        .and_then(|p| p.get("name"))
                        .and_then(|n| n.as_str())
                        == Some("bevy_rui")
                    {
                        let ver = val
                            .get("package")
                            .and_then(|p| p.get("version"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        return (
                            Some(dir.to_path_buf()),
                            ver,
                            Some(format!("{}", toml_path.display())),
                        );
                    }

                    // 2. Check dependencies / workspace.dependencies for `bevy_rui`
                    for table_entry in [
                        val.get("dependencies"),
                        val.get("dev-dependencies"),
                        val.get("workspace").and_then(|w| w.get("dependencies")),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if let Some(dep) = table_entry.get("bevy_rui") {
                            if let Some(ver) = dep.as_str() {
                                return (
                                    None,
                                    Some(ver.to_string()),
                                    Some(format!("{} (dependencies.bevy_rui)", toml_path.display())),
                                );
                            }
                            let dep_path = dep
                                .get("path")
                                .and_then(|p| p.as_str())
                                .map(|p| dir.join(p));
                            let dep_ver = dep
                                .get("version")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            if dep_path.is_some() || dep_ver.is_some() {
                                return (
                                    dep_path,
                                    dep_ver,
                                    Some(format!("{} (dependencies.bevy_rui)", toml_path.display())),
                                );
                            }
                        }
                    }
                }
            }
        }

        let lock_path = dir.join("Cargo.lock");
        if lock_path.is_file() {
            if let Ok(text) = fs::read_to_string(&lock_path) {
                if let Ok(val) = text.parse::<toml::Value>() {
                    if let Some(pkgs) = val.get("package").and_then(|p| p.as_array()) {
                        for pkg in pkgs {
                            if pkg.get("name").and_then(|n| n.as_str()) == Some("bevy_rui") {
                                if let Some(ver) = pkg.get("version").and_then(|v| v.as_str()) {
                                    return (
                                        None,
                                        Some(ver.to_string()),
                                        Some(format!("{}", lock_path.display())),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        cur = dir.parent();
    }
    (None, None, None)
}

fn load_config_file(project_root: &Path) -> (Option<PathBuf>, ConfigFile) {
    let mut candidates = Vec::new();
    if let Ok(cfg_env) = env::var("BEVY_RUI_MCP_CONFIG") {
        candidates.push(expand_home(&cfg_env));
    }
    candidates.push(project_root.join("bevy-rui-mcp.config.json"));
    if let Ok(exe) = env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("bevy-rui-mcp.config.json"));
            if let Some(parent) = exe_dir.parent() {
                candidates.push(parent.join("bevy-rui-mcp.config.json"));
                if let Some(grand) = parent.parent() {
                    candidates.push(grand.join("bevy-rui-mcp.config.json"));
                }
            }
        }
    }
    if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
        candidates.push(
            PathBuf::from(home)
                .join(".config")
                .join("bevy-rui-mcp")
                .join("config.json"),
        );
    }

    for candidate in candidates {
        if candidate.is_file() {
            if let Ok(content) = fs::read_to_string(&candidate) {
                if let Ok(parsed) = serde_json::from_str::<ConfigFile>(&content) {
                    return (Some(candidate), parsed);
                }
            }
        }
    }
    (None, ConfigFile::default())
}

/// Newest `bevy_rui` version already installed as a persisted index under `<data_dir>/versions/*`.
///
/// Last-resort version resolution, so a source-free setup (bundle downloaded from a GitHub Release,
/// with no `bevy_rui` checkout, no config file, no Cargo dependency) still knows which `bevy_rui`
/// version it is serving: only the package name — implied by this MCP — plus the installed version
/// are needed. Picks the numerically newest dotted version whose index files actually exist, rather
/// than the lexicographically last registry key (which would rank `0.1.10` below `0.1.7`).
pub fn persisted_index_version(data_dir: &Path) -> Option<String> {
    let reg = read_registry(data_dir);
    let mut best: Option<String> = None;
    let mut best_key: Vec<u64> = vec![];
    for (ver, _) in reg.versions.iter() {
        let vdir = version_dir(data_dir, &ver);
        if !vdir.join("records.ndjson").is_file() || !vdir.join("meta.json").is_file() {
            continue;
        }
        let mut key: Vec<u64> = vec![];
        for part in ver.split(".") {
            key.push(part.parse::<u64>().unwrap_or(0));
        }
        if best.is_none() || key > best_key {
            best = Some(ver.clone());
            best_key = key;
        }
    }
    best
}

pub fn resolve_config(overrides: &CliOverrides) -> ResolvedConfig {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let initial_root = overrides
        .project_root
        .clone()
        .or_else(|| env::var("BEVY_RUI_PROJECT_ROOT").ok().map(|s| expand_home(&s)))
        .unwrap_or_else(|| cwd.clone());

    let (config_path, file_cfg) = load_config_file(&initial_root);
    let cfg_base = config_path
        .as_deref()
        .and_then(|p| p.parent())
        .unwrap_or(&initial_root);

    let project_root = overrides
        .project_root
        .clone()
        .or_else(|| env::var("BEVY_RUI_PROJECT_ROOT").ok().map(|s| expand_home(&s)))
        .or_else(|| {
            file_cfg
                .project_root
                .as_deref()
                .map(|p| resolve_rel_path(p, cfg_base))
        })
        .unwrap_or_else(|| initial_root.clone());

    let (cargo_rui_path, cargo_rui_ver, cargo_source) = detect_from_cargo_project(&project_root);

    // Resolve rui_path in priority order:
    // 1. CLI --rui-path
    // 2. BEVY_RUI_PATH env var
    // 3. config file `ruiPath`
    // 4. Cargo.toml path dependency / current dir if it is bevy_rui
    // 5. Generic sibling/ancestor locations named `bevy_rui` (`./bevy_rui`, `../bevy_rui`, `../../bevy_rui`,
    //    and the same shapes around the cwd and the executable's ancestors). Discovery is name-based
    //    only: no author-specific checkout location is encoded here.
    let mut rui_path: Option<PathBuf> = None;
    let mut rui_path_source = "not found (using prebuilt index if installed)".to_string();

    if let Some(p) = &overrides.rui_path {
        if is_bevy_rui_dir(p) {
            rui_path = Some(fs::canonicalize(p).unwrap_or_else(|_| p.clone()));
            rui_path_source = "--rui-path CLI flag".to_string();
        }
    }
    if rui_path.is_none() {
        if let Ok(env_p) = env::var("BEVY_RUI_PATH") {
            let p = expand_home(&env_p);
            if is_bevy_rui_dir(&p) {
                rui_path = Some(fs::canonicalize(&p).unwrap_or(p));
                rui_path_source = "BEVY_RUI_PATH env var".to_string();
            }
        }
    }
    if rui_path.is_none() {
        if let Some(cfg_p) = &file_cfg.rui_path {
            let p = resolve_rel_path(cfg_p, cfg_base);
            if is_bevy_rui_dir(&p) {
                rui_path = Some(fs::canonicalize(&p).unwrap_or(p));
                rui_path_source = format!(
                    "config file ({})",
                    config_path
                        .as_ref()
                        .map(|c| c.display().to_string())
                        .unwrap_or_default()
                );
            }
        }
    }
    if rui_path.is_none() {
        if let Some(p) = cargo_rui_path {
            if is_bevy_rui_dir(&p) {
                rui_path = Some(fs::canonicalize(&p).unwrap_or(p));
                rui_path_source = cargo_source.clone().unwrap_or_else(|| "Cargo.toml".to_string());
            }
        }
    }
    if rui_path.is_none() {
        let mut sibling_candidates = vec![
            project_root.clone(),
            project_root.join("bevy_rui"),
            project_root.join("..").join("bevy_rui"),
            project_root.join("..").join("..").join("bevy_rui"),
            cwd.join("bevy_rui"),
            cwd.join("..").join("bevy_rui"),
            cwd.join("..").join("..").join("bevy_rui"),
        ];
        if let Ok(exe) = env::current_exe() {
            for ancestor in exe.ancestors().skip(1).take(4) {
                sibling_candidates.push(ancestor.join("bevy_rui"));
                sibling_candidates.push(ancestor.join("..").join("bevy_rui"));
            }
        }
        for cand in sibling_candidates {
            if is_bevy_rui_dir(&cand) {
                rui_path = Some(fs::canonicalize(&cand).unwrap_or(cand));
                rui_path_source = "auto-discovered checkout".to_string();
                break;
            }
        }
    }

    let (manifest_rui_ver, manifest_bevy_ver) = rui_path
        .as_deref()
        .map(read_rui_manifest_info)
        .unwrap_or((None, None));

    // Version is resolved *after* `data_dir` (below): a source-free setup, which has no checkout and
    // no config file to read, derives the `bevy_rui` version from the prebuilt index installed under
    // `<data_dir>/versions` instead of silently claiming this MCP's own package version.

    // Resolve data_dir
    let data_dir = overrides
        .data_dir
        .clone()
        .or_else(|| env::var("BEVY_RUI_DATA_DIR").ok().map(|s| expand_home(&s)))
        .or_else(|| {
            file_cfg
                .data_dir
                .as_deref()
                .map(|p| resolve_rel_path(p, cfg_base))
        })
        .unwrap_or_else(|| {
            if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
                PathBuf::from(home)
                    .join(".cache")
                    .join("bevy-rui-mcp")
                    .join("data")
            } else {
                project_root.join(".bevy-rui-mcp-data")
            }
        });

    let (rui_version, version_source) = if let Some(v) = &overrides.version {
        (v.clone(), "--version CLI flag".to_string())
    } else if let Ok(v) = env::var("BEVY_RUI_VERSION") {
        (v, "BEVY_RUI_VERSION env var".to_string())
    } else if let Some(v) = &file_cfg.version {
        (v.clone(), "config file".to_string())
    } else if let Some(v) = manifest_rui_ver {
        (v, "bevy_rui/Cargo.toml".to_string())
    } else if let Some(v) = cargo_rui_ver {
        (v, cargo_source.unwrap_or_else(|| "Cargo.toml".to_string()))
    } else if let Some(v) = persisted_index_version(&data_dir) {
        (v.clone(), "installed prebuilt index (<data_dir>/versions)".to_string())
    } else {
        (env!("CARGO_PKG_VERSION").to_string(), "default package version".to_string())
    };

    let repo = overrides
        .repo
        .clone()
        .or_else(|| env::var("BEVY_RUI_REPO").ok())
        .or_else(|| file_cfg.repo.clone())
        .unwrap_or_else(|| "prfiredragon/bevy_rui_mcp".to_string());

    let max_results = env::var("BEVY_RUI_MAX_RESULTS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .or(file_cfg.max_results)
        .unwrap_or(10)
        .clamp(1, 25);

    ResolvedConfig {
        project_root,
        rui_path,
        rui_path_source,
        rui_version,
        version_source,
        bevy_version: manifest_bevy_ver,
        data_dir,
        repo,
        max_results,
        config_file: config_path,
    }
}
