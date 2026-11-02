# TODO validation log — 2026-11-01

Verdicts on each item of `TODO.md`, established by running the code (repo read-only;
rebuild/repair work was done in a scratch copy under `$JCODE_SCRATCH_DIR`).

Legend: ✅ validated accurate · ⚠️ accurate but incomplete / needs correction · ❌ wrong.

---

## Item 0 — compile errors in `src/ingest.rs` (10) → ✅ validated

`cd /home/robert/rdev/bevy_rui_mcp && cargo build` reproduces exactly **10 errors**, one per
documented location: `266:18`, `266:30`, `280:30`, `280:50`, `307:33`, `315:14`, `317:56`,
`672:47`, `936:54`, `945:59`. All are in `src/ingest.rs`; nothing else in the tree fails to compile.

The suggested fixes are right for #1–#8 and #9/#10 individually, **but applying the table verbatim
does not compile**: three new errors appear that the table does not list.

| surfaced after table fixes | where | cause |
|---|---|---|
| E0507 ×2 | `936:54` / `945:59` | `rec.owner` / `decl.owner` are `Option<String>` reached through a reference — `unwrap_or("".to_string())` still moves out from behind a borrow; needs `(*x.owner).clone()`-style handling or restructuring the loop |
| E0502 | iter at `313:49` | nested borrow of `records` while mutating — the `.map(\|w\| w.clone())` fix needs an index-based loop |

Verified fix path: the scratch copy `validation_buildfix` compiles clean (`cargo build`, 2.79 s,
3 warnings) after the table fixes **plus** the borrow/ownership adjustments above.

Item 0's "also confirm" checks: ✅ `widget_from_decl_file` is gone (no occurrence in `src/ingest.rs`);
✅ no orphan `///` block — the doc comment at lines 236–245 belongs to `widget_names_in_checkout`
(line 246) and reads coherently.

---

## Item 1 — association strategy unverified → ✅ validated as OPEN, with one correction

The repo's committed index (`data/versions/0.1.7/records.ndjson`) does **not** implement the
association: `widget_from` distribution is only `{None: 110, path: 152}`, and
`RuiBuilderExt::vbox`, `::hbox`, `::window`, `::menu_bar`, `::multiline_textbox` all have
`widget: null`. So the feature is genuinely unverified in the shipped data — item 1 stands.

Rebuilding in the scratch copy from `<the previous author's local bevy_rui checkout>` gives:

```
records=262  widgets=24  cache_version=2  symbols=910
widget_from: {None: 56, path: 152, doc_link: 27, decl_trait: 27}   bogus widget refs: 0
```

Per acceptance check:

| check | verdict | evidence |
|---|---|---|
| 262 records, +42 `trait_impl_method`, `implements` 33→75 | ✅ | 262 records; `trait_impl_method` = 42; 75 `implements` entries |
| `windows` present, no synthetic `window` | ✅ | 24 widgets: accordion … viewport, `windows`; no `window` |
| `RuiBuilderExt::window` → `windows` / `doc_link` | ✅ | `widget=windows from=doc_link`, doc link `windows::spawn_window` |
| `vbox`, `hbox` → `layout` | ✅ | both `doc_link` (declarations), `decl_trait` (impl bodies) |
| **`rui_root` → `layout`** | ⚠️ **correction** | only the impl body gets it: `widgets::layout::RuiRootBuilderExt::Commands::rui_root` → `layout` / `path`. The declaration `widgets::RuiRootBuilderExt::rui_root` stays `widget: null` — its doc text names no widget, so the "no invented widget" rule in the very next check forbids associating it. As written this acceptance line cannot pass; reword to "`rui_root` impl body → `layout`; declaration may stay `null`". |
| `multiline_textbox` → `textbox` | ✅ | `widget=textbox from=doc_link` |
| menu builder methods → `menu` | ✅ | `menu_bar`, `menu_item`, `submenu` → `menu` |
| `button` → `button` | ✅ | `widget=button from=path` |
| builder methods appear in widget guide | ✅ | `rui_widget windows` lists `RuiBuilderExt::window` (trait_method) + `…::ChildSpawnerCommands::window` (trait_impl_method); `layout` shows `vbox`/`hbox`/`rui_root`; `menu` shows the three menu methods |
| docs naming no widget stay `null` | ✅ | 56 records `null`; no name-equality guessing observed |
| `trait_impl_method` inherit via declaring trait | ✅ | 27 records `widget_from: "decl_trait"` |

Cosmetic gap not in TODO: `widget_provenance()` in `src/tools.rs` maps `"name"`, `"owner"`, `"reexport"`
but has no case for the new `"doc_link"` / `"decl_trait"` keys, so guides print the raw key
(`**Widget:** \`windows\` (doc_link)`). Same for the doc comment on `widget_from` in `src/types.rs`.

---

## Item 2 — source-free serving → ✅ OPEN (mechanism works, version provenance does not)

Reproduced with the scratch binary and an isolated data dir containing only
`registry.json` + `versions/0.1.7/{meta.json,records.ndjson}`:

```
rui_path      : (none) (not found (using prebuilt index if installed))
rui_version   : 0.1.7 (default package version)
index on disk : yes (.../versions/0.1.7)
```

Also reproduced with a relocated binary run from `/tmp` — discovery still resolves the persisted index,
exit 0. So the "already confirmed working once" claim is reproducible.

The unchecked half is accurate: `src/config.rs:369` is the last branch of the version ladder,
`(env!("CARGO_PKG_VERSION"), "default package version")` — i.e. `0.1.7` is compiled in, and the
persisted registry/`versions/*` dir is never consulted for the version. Nothing visibly distinguishes
"from persisted index" vs "from source tree". Item 2 stands.

Caveat on the "already confirmed working" claim: source-free is only true while the process's cwd and
its executable's ancestors are **outside** `/home/robert/rdev`. Run from inside that tree, discovery
reattaches `<the previous author's local bevy_rui checkout>` (see item 4) — so the claim is
"working in a neutral location", not "independent of the source tree".

---

## Item 3 — `fetch-index` / dist bundle → ✅ validated, plus 2 facts the TODO misses

Stale bundle confirmed (`dist/index-manifest.json`, mtime Oct 28 14:46):

```
cache_version: 1, total_records: 219, asset: bevy-rui-index-0.1.7.tar.gz (47962 B),
sha256: 8afe7d1f…
```

Reproduced failure: `fetch-index --from dist/bevy-rui-index-0.1.7.tar.gz --version 0.1.7` →
`Persisted index cache version mismatch (found v1, expected v2)` (the loader hard-fails; `types.rs:75`
`CACHE_VERSION = 2`). The v1 bundle also disagrees with the current index content-wise: 219 records vs
262, and 25 widgets vs 24 (it carries the synthetic `window`).

Facts the TODO does not state:
- **`/dist` and `/data` are `.gitignore`d (lines 2–3) and untracked**, so no index artifact exists in
  the repo or in the single commit `96d3796` — the "download from GitHub Releases" premise currently has
  nothing to download. Ship-ready bundles must be force-added or published out-of-band.
- **The regeneration path already works.** In the scratch copy: `publish-index` emits
  `cache_version: 2 / total_records: 262 / 24 widgets` (sha `4c0c70ab…`), and
  `fetch-index --from <that tar.gz>` installs it, after which `status` in a source-free cwd reports
  `index on disk: yes`. So item 3 is "regenerate + stop gitignoring + migrate v1", not "redesign".
- README's documented asset name is `bevy-rui-index-v0.1.7.tar.gz` (with `v`), the code emits
  `bevy-rui-index-0.1.7.tar.gz` (no `v`) — doc/CLI mismatch that also broke copy-paste of the README
  `fetch-index` example.

---

## Item 4 — hard-coded author layout → ✅ validated

`src/config.rs` occurrences of `<author's game project>`: **lines 332, 335, 340** (plus a comment at 285).
Empirically, running the binary from a scratch cwd reattached
`<the previous author's local bevy_rui checkout>`; the persisted index metadata itself records the
author path (`registry.json` → `rui_path`, `meta.json` → `rui_path`), so a downloaded bundle carries
that path into the user's status output. Precedence today (config.rs:252 → resolve_config):
CLI > `BEVY_RUI_*` env > config file > `Cargo.toml`/`Cargo.lock` path dep > sibling `bevy_rui`/`../bevy_rui`
> filesystem walk of the parent tree — **persisted index is never consulted for the version** and
discovery is not suppressed when an index is present. Item 4's three boxes are all unimplemented.

---

## Item 5 — stale "21 widgets" wording → ✅ validated, exact sites

Only two baked-in counts exist repo-wide in `src/`:

- `src/tools.rs:128` — "…list of all 21 widgets if `widget` is omitted"
- `src/mcp.rs:81` — "…inspect any of the 21 widgets…"

Dynamic rendering is already in place elsewhere (`src/tools.rs:590` "Widgets ({})",
`src/tools.rs:884` "Widgets ({}):"), and probes confirm live output renders `Widgets (24)`.
All 24 registry widgets resolve and render a guide (0 unknown, 0 without "Widget Guide"), so
"21" is simply stale in both sites → 24.

Also found (not in item 5): the only other baked-in version-ish literal is `env!("CARGO_PKG_VERSION")`
(`src/config.rs:369`), i.e. the fallback version is the **MCP package's** version (`0.1.7`), which today
coincidentally equals the indexed crate's version — this is the coupling behind item 2/4's
"compiled-in default" symptom, and it changes whenever this package is bumped.
Docs-only occurrences: `src/config.rs:43` example comment.

---

## Item 6 — README docs → ⚠️ the requested section already exists; README has other drift

Already present: `README.md:71` "### 3. Descargar e instalar un índice precompilado (GitHub Releases
o archivo/URL)" with `--repo …/bevy_rui_mcp --version 0.1.7` and a `--from ./dist/…` variant, plus
`README.md:121` "Opción B: Usando variables de entorno" (`BEVY_RUI_DATA_DIR`) and an MCP client config
block with `"serve", "--data-dir"`. So the "no checkout needed" narrative is **already covered**; item 6
is not a missing-docs task, it is a doc-accuracy task. Real README drift found instead:

| README | says | actual |
|---|---|---|
| 5, 147 | theming type `UiTheme` | indexed symbol is `bevy_rui::theme::RuiTheme` (+ `RuiThemeElement`) |
| 152–154 | resources `bevy-rui://status`, `bevy-rui://widgets`, `bevy-rui://symbol/{path}` | server registers/emits `rui://` scheme only: `rui://widgets`, `rui://widget/{name}`, `rui://symbol/{name}`, `rui://example/{name}` (`src/mcp.rs:121,129,137,159,165,171`); `bevy-rui://…` → `Unknown resource URI`, and there is **no** `rui://status` resource (status is a tool only) |
| 63, 83 | asset `bevy-rui-index-v<version>.tar.gz` | `publish-index` emits `bevy-rui-index-<version>.tar.gz` (no `v`), e.g. `bevy-rui-index-0.1.7.tar.gz` |
| 121+ (Opción B) | env vars `BEVY_RUI_PATH`, `BEVY_RUI_DATA_DIR` only | code honors 7: also `BEVY_RUI_VERSION` (config.rs:360), `BEVY_RUI_REPO` (397; default `prfiredragon/bevy_rui_mcp`, main.rs:38), `BEVY_RUI_MAX_RESULTS` (401), `BEVY_RUI_PROJECT_ROOT` (257/269), `BEVY_RUI_MCP_CONFIG` (216) — 5 of 7 undocumented |
| — | — | nothing documents that `/data` and `/dist` are `.gitignore`d, which contradicts the GitHub-shipping story |

---

## Item 7 — cleanup strays → ✅ validated

`git status --porcelain` (single commit `96d3796 first commit`, so *everything* except those 5 modified
`src/*.rs` is "first commit" baseline):

```
 M src/ingest.rs   M src/mcp.rs   M src/store.rs   M src/tools.rs   M src/types.rs
?? TODO.md  ?? jcode_probe_build_err.txt  ?? package-lock.json  ?? package.json
```

- `package.json` = 3 bytes, `{}`; `package-lock.json` = 91 bytes, empty lockfile (no packages) —
  confirmed junk from an accidental Node run, no Rust-side consumer, safe to delete.
- `jcode_probe_build_err.txt` = 4847 B (Oct 30 18:06) — stale build-error capture; its contents
  (`E0063` missing-field errors) do **not** match the current 10-error set, so it is a probe artifact,
  not the present failure mode.
- `.gitignore` (54 B, 5 entries): `/target`, `/dist`, `/data`, `*.tar.gz`,
  `bevy-rui-mcp.config.json` — `/dist` + `/data` must go for the prebuilt-index-from-GitHub story to be
  self-contained (the tracked `*.example.json` config file is fine; the real one is ignored by design).

---

## Reference-block claims in `TODO.md` → one is stale

- "JSON-RPC stdio probe harness … still valid" → ⚠️ partially. The harness works for tool calls, but it
  reads `t.get("parameters")` while the server emits `inputSchema` (`src/tools.rs` / MCP `tools/list`),
  so it never validated schemas; also it omits `clientInfo`/`capabilities`-style fields unless patched,
  and `protocolVersion` falls back to `2024-11-05`. Re-patch before trusting "valid".
- "Modified since `96d3796`: `src/ingest.rs`, `src/mcp.rs`, `src/store.rs`, `src/tools.rs`,
  `src/types.rs`" → ✅ exact.
- "Scratch dir is available; large temp files go under `$JCODE_SCRATCH_DIR`, not `/tmp`" → used
  `/home/robert/.jcode/scratch` throughout (`validation_buildfix`, `validation_srcfree`, `validation_fetch`,
  `validation_dl`).
- Rebuild command in item 1 uses `./target/release/bevy-rui-mcp`; that binary is **stale** (Oct 31 17:22,
  built from an older source state) — rebuild before using it.

---

## Corrections to make in TODO.md (validated deltas)

1. **Item 0**: the fix table gets the build compiling to within 3 errors; add the follow-on set
   (E0507 ×2 at the `owner`/`decl.owner` sites, E0502 nested-borrow at the `collect()` site) —
   `unwrap_or("")` alone does not fix #9/#10 because the option is reached through a borrow.
2. **Item 1, `rui_root -> layout`**: as literally worded it is unachievable under the next acceptance
   bullet ("methods whose docs name no widget stay `widget: null`"). Observed truth:
   impl body → `layout`/`path`; trait declaration `widgets::RuiRootBuilderExt::rui_root` → `null`.
   Reword the bullet, or accept the exception explicitly.
3. **Item 1, `implements 33 -> 75`**: already 75 in the checked-in `meta.json` (and `total_records` 262,
   `trait_impl_method` 42) — the checked-in index is *post*-expansion in counts but *pre*-association
   (`widget_from` ∈ {null, path} only). State it that way to avoid chasing a phantom 220→262 jump.
4. **Item 3**: add the two blockers the TODO omits — `/dist` + `/data` are gitignored/untracked (nothing
   is actually published), and the README asset name (`…-v0.1.7.tar.gz`) ≠ emitted name (`…-0.1.7.tar.gz`).
5. **Item 6**: replace "add a section" with "fix README drift": `UiTheme`→`RuiTheme`,
   `bevy-rui://`→`rui://` (and add the missing `rui://widget/…`, `rui://example/…`; drop the nonexistent
   `status` resource), asset-name pattern, missing `BEVY_RUI_*` env vars.
