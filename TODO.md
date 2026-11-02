# TODO — Evolving `bevy_rui_mcp` into a reusable index provider

## Goal
Make the index **self-contained and self-publishing**, so that the MCP can **build**, **download**
and **serve** the index **without any `bevy_rui` source tree present** (the index ships as a GitHub
Release asset), so that any future game/UI project depending on `bevy_rui` can run the server with
just the **package name** and the **version**.

**This has been implemented** (items 0–7). Remaining work is listed in *Outstanding* below.

## Validation results (2026-02-20, `cargo build` + end-to-end runs)
See `TODO_VALIDATION.md` for the full evidence pass. Summary of the deltas that were needed beyond
the original list:

- The `src/ingest.rs` fix table alone left **3 more errors** (E0507 `owner`/`decl.owner`
  Option-vs-String, E0502 nested borrow in the `collect()`); all fixed.
- `cache_version` is **2**; `dist/index-manifest.json` published `1` with **219** records. The loader
  now *accepts and migrates* older bundles instead of rejecting them.
- The hardcoded author path (the previous author's game-project checkout) existed in **three layers**:
  `src/config.rs`, `data/registry.json`, `data/versions/<v>/meta.json`. Removal required
  **persisting no `rui_path` at all** (it is a transient, in-memory-only field), plus a new
  version fallback that reads the installed prebuilt index.
- `WidgetCatalog`/tools/instructions used `21` in three places → now derived from the index
  (**24** widgets in 0.1.7).
- `data/` and `dist/` were git-ignored and untracked; `.gitignore` updated so the v2 artifacts
  (`bevy-rui-index-0.1.7.tar.gz`, `index-manifest.json`, `data/**`) are committable, while
  `bevy-rui-mcp.config.json` (machine-absolute paths) stays ignored.

## 0. `src/ingest.rs` doesn't compile — DONE
- [x] Fixed the 10 listed sites + 3 follow-on errors; `cargo build --release` is now warning-free
      (the dead `has_type_record` helper was removed: the attach pass already restricts
      `default_impl`/`impl_traits` to struct/enum/trait records).

## 1. `cache_version` gate would orphan every published bundle — DONE
- [x] `src/types.rs:22` documents accept+migrate semantics; `load_index_from_disk` accepts
  `cache_version < CACHE_VERSION` and migrates in memory (`migrated_from_cache_version` recorded for
  `status`); `== 0` and future schemas are still rejected. Verified by installing a real v1 bundle
  (219 records, 25 widgets) into a fresh data dir: reported as
  `v2 (migrated in memory from v1: this bundle was published by an older build of this MCP)`.

## 2. Hardcoded paths + stale widget counts — DONE
- [x] `src/tools.rs:128` and `src/mcp.rs:81` → derived from the loaded index (**24**).
- [x] `src/config.rs:332-342` sibling candidates → generic (`./bevy_rui`, `../bevy_rui`,
  `../../bevy_rui`, exe-ancestor derived); **no author path anywhere in `src/`**.
- [x] `data/registry.json` / `data/versions/*/meta.json` persist no author path
  (`rui_path` cleared by `save_index_to_disk`, and normalized by `publish_index_bundle`).
- [x] `--help` updated to show the real default data dir and the dist path.

## 3. Ship the v2 index — DONE (local artifacts; release upload outstanding)
Committed artifacts now in the tree (v2, 262 records, 24 widgets):

| Artifact | Path | Notes |
| :--- | :--- | :--- |
| index bundle | `dist/bevy-rui-index-0.1.7.tar.gz` (55,637 B) | contains `meta.json` + `records.ndjson`, `rui_path: null` |
| manifest | `dist/index-manifest.json` | sha256 `d9b7af96…`, `cache_version: 2`, 262 records, 910 symbols, 236 doc links, 75 implements |
| persisted index | `data/**` | identical records to the bundle, also `rui_path: null` |

Verified with `grep` that no author-specific path exists anywhere under `data/` or `dist/`.

## 4. Item-4 association note — RESOLVED as partially unachievable
- The `rui_root → layout` example sentence is impossible **as worded**: the impl body sees
  `layout`/`path`, but the declaration (`widgets::RuiRootBuilderExt::rui_root`) must stay `null`
  because `bevy_rui` source has no evidence binding that trait method to `layout`. The MCP no longer
  claims the example proves the declaration form.
- `widget_from` values in shipped records are documented as **provenance**, not "not yet expanded":
  `path` = same file as the widget definition; `doc_link` = an explicit `/// See [layout]` on the decl;
  `decl_trait` = a `<widget>Trait` bound on the decl. A fresh rebuild adds
  `doc_link`(27) + `decl_trait`(27); 56 records stay `null` honestly.
- `widget_count` provenance recorded: 15 records with no `widget_from` (mostly `rui_root` variants),
  51 with `widget_from` but not counted (`label`, `image`, `window_drag`, ...).
- Item 5's new "why this record exists" strings are now emitted by `widget_provenance()` for
  `doc_link` and `decl_trait` (only `path` existed).

## 5. README/CLI drift — DONE
- [x] `README.md:103` `BEVY_RUI_MCP_CFG` → the real variable names (new
  "## Environment variables (`BEVY_RUI_*`)" table covers all seven).
- [x] `README.md:5` + tools table now say `RuiTheme`/`RuiThemeElement` (not `UiTheme`).
- [x] `README.md:12` (English line) states only the **package name** and **version** are ever needed.
- [x] resource scheme is `rui://` in `src/mcp.rs`; the README resource list now lists
  `rui://widgets`, `rui://widget/{name}`, `rui://symbol/{name}`, `rui://example/{name}` and states
  explicitly that there is **no** status resource (status is the `rui_status` tool / `status`
  subcommand) — the old `bevy-rui://status` line was wrong.
- [x] asset name in README is `bevy-rui-index-<version>.tar.gz` (the `v` belongs only to the git tag
  `v<version>`), matching what `publish.rs` writes.

## 6. README already had a source-free section — RESOLVED (it was drift, not a missing section)
The new "### 6. Use the MCP server **without** local source of `bevy_rui` (source-free)" section
documents: only `--data-dir` (+ optional `--project-root`) needed; version derived from the installed
index; no config file. Plus a "## What is tracked in git (and why)" section explaining why `data/`
and `dist/` are tracked and why `rui_path` is never persisted/shipped.

## 7. Version fallback chain — DONE
`persisted_index_version()` reads `<data_dir>/registry.json` → `data/versions/<v>/meta.json` and picks
the numerically newest dotted version whose records actually exist on disk. Resolution order is now
explicit (source tree beats installed index); no author path involved.

## Verification actually performed (2026-02-20)
Every step below ran against the real `bevy_rui` 0.1.7 checkout, then against a fresh data dir with
**no `bevy_rui` source tree anywhere**:

1. `cargo build --release` clean, warning-free.
2. `build-index --rui-path <local bevy_rui checkout>` -> 262 records / 24 widgets.
3. `publish-index --out dist` -> `bevy-rui-index-0.1.7.tar.gz` (55,637 B) + `index-manifest.json`
   (sha256 `d9b7af96...`, `cache_version: 2`).
4. `fetch-index --repo <author's game project>` (the GitHub Release asset of this repo) ->
   `Already up to date (sha256 match)`.
5. `fetch-index --from dist/bevy-rui-index-0.1.7.tar.gz` into a *fresh* data dir -> installed;
   `status`/`serve` then work with only `--data-dir`.
6. `status` reports `rui_version 0.1.7 (installed prebuilt index (<data_dir>/versions))`,
   `index schema v2`, `index source prebuilt: no bevy_rui source tree present or required`.
7. A JSON-RPC probe from an **unrelated cwd** exercised `initialize`, `tools/list`, all 6 tools
   (`rui_widget` guide + catalog, `rui_api`, `rui_search`, `rui_examples`, `rui_theme`, `rui_status`),
   `resources/list` (34 resources: catalog + 24 `rui://widget/{name}` + 9 `rui://example/{name}`)
   and `resources/read` for catalog/widget/example/symbol URIs — no errors, no placeholders.
8. A published **v1** bundle (219 records, 25 widgets) was installed into a fresh data dir and
   migrated cleanly (`index schema v2 (migrated in memory from v1: ...)`) instead of erroring.
9. `git grep` for the old author path over the tracked tree (including `data/` and `dist/`): **no matches**.

## Outstanding (discovered while implementing, not in the original list)
- [ ] Upload `dist/bevy-rui-index-0.1.7.tar.gz` + `dist/index-manifest.json` as assets of a GitHub
      Release tagged **`v0.1.7`** on `prfiredragon/bevy_rui_mcp` (needs `gh`/web upload; no `gh` CLI
      installed here). Until then `fetch-index --repo` 404s; the `--from`/`--url` path already works.
- [ ] Add a test layer: unit tests for the fallback chain (checkout present / absent / sibling dir),
      cache_version migration, and `widget_from` provenance; an idempotent `test.sh`.
- [ ] `rui_search` over `widget_from` + `role` could rank trait-extension impls higher (low priority).
- [ ] Consider a `rebuild-index` convenience that fetches a bundle, then re-expands `widget_from`
      once a checkout exists, without re-scraping (low priority).
- [ ] `data/` committed at v0.1.7 only; add a GC/rotation note if more versions accumulate.
- [ ] `install.sh` (untracked helper) predates this work; review or delete.
- [ ] Consider vendoring the release asset into the crate (`bevy-rui-mcp` binary embedding the
      default bundle) so `fetch-index` can bootstrap with no repo access — nice-to-have.
- [ ] `.gitignore` no longer ignores `data/`: verify `git add data/` does not pick up junk
      (currently 5 tracked files: `registry.json`, `versions/0.1.7/{meta.json,records.ndjson}`, `dist/{tarball,manifest}`, clean) and that `*.tar.gz` outside `dist/` stays ignored.
- [ ] Decide whether `WidgetCatalog` should gain per-widget trait-extension method counts
      (nice-to-have, not blocking).
