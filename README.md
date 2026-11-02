# bevy_rui_mcp

Servidor MCP (Model Context Protocol) **100% nativo en Rust** diseñado exclusivamente para la librería de UI [`bevy_rui`](https://github.com/seregonwar/bevy_rui).

Analiza directamente el árbol sintáctico (AST con `syn`) de `bevy_rui` —widgets, componentes, traits de extensión (`RuiBuilderExt`, `RuiRootBuilderExt`, `TooltipExt`), temas (`RuiTheme`/`RuiThemeElement`), sistema de foco/navegación y todos los ejemplos (`examples/*.rs`)— y los expone mediante búsqueda híbrida en memoria y herramientas MCP por `stdio`.

---

## Características principales

- **100% Rust nativo**: Binario único (`bevy-rui-mcp`), sin dependencias de Node.js ni Python.
- **Rutas flexibles (No requiere rutas estándar)**: Funciona sin importar dónde estén clonados `bevy_rui` o `bevy_rui_mcp`, o incluso sin el código fuente de `bevy_rui` usando índices precompilados. Only the **package name** (`bevy_rui`, implied by this server) and the **version** are ever needed: no path specific to a developer is hardcoded in the code, in the data committed to the repository, or in the published bundles.
- **Indexación AST directa (`syn`)**:
  - Extrae `struct`s, campos documentados, `enum`s, variantes, funciones `spawn_*` (con su cuerpo e implementación real), `trait`s, métodos e implementaciones de `Default`/`FromWorld`.
  - Analiza todos los ejemplos (`examples/*.rs`) y detecta automáticamente qué widgets usa cada ejemplo.
- **Soporte para GitHub Releases (`publish-index` / `fetch-index`)**:
  - Empaqueta el índice en `.tar.gz` + `index-manifest.json` con SHA-256 para subirlo a GitHub Releases.
  - Descarga e instala índices precompilados directamente desde GitHub Releases o desde una URL/archivo local.

---

## Compilación

```bash
cargo build --release
```

El binario quedará en `target/release/bevy-rui-mcp`.

---

## Resolución de rutas (Flexible Path Support)

Como `bevy_rui` puede estar ubicado en cualquier directorio elegido por el usuario, `bevy-rui-mcp` resuelve la ruta al código fuente de `bevy_rui` en este orden de prioridad:

1. **Flag CLI**: `--rui-path /ruta/a/bevy_rui`
2. **Variable de entorno**: `BEVY_RUI_PATH=/ruta/a/bevy_rui`
3. **Archivo de configuración**: `bevy-rui-mcp.config.json` (campo `"ruiPath"`)
4. **Auto-detección desde el proyecto (`--project-root` o directorio actual)**:
   - Si el directorio actual es un checkout de `bevy_rui` (`Cargo.toml` con `name = "bevy_rui"`).
   - Si el `Cargo.toml` del proyecto del usuario tiene `bevy_rui = { path = "..." }`.
   - Si existe a sibling directory `./bevy_rui`, `../bevy_rui` or `../../bevy_rui` (also searched around the current directory and around the executable's ancestors). Auto-detection is always **by package name**: no location specific to an author is used.
5. **Already installed precompiled index** (`<data-dir>/versions/<version>/`): when there is no local checkout and no `Cargo.toml`/`Cargo.lock` reference, the `bevy_rui` **version** is taken from the index persisted on disk (numerically newest), so a project without sources needs neither `--version`, `BEVY_RUI_VERSION`, nor `bevy-rui-mcp.config.json`.

El directorio donde se guardan los índices (`data/`) también es configurable mediante:
1. `--data-dir /ruta/a/data`
2. `BEVY_RUI_DATA_DIR=/ruta/a/data` (default if not specified: `~/.cache/bevy-rui-mcp/data`)
3. `bevy-rui-mcp.config.json` (campo `"dataDir"`)

---

## Uso por CLI

### 1. Construir el índice desde un directorio local de `bevy_rui`

```bash
bevy-rui-mcp build-index \
  --rui-path /ruta/donde/clonaste/bevy_rui \
  --data-dir /ruta/donde/guardar/data
```

### 2. Empaquetar el índice para GitHub Releases

Genera `bevy-rui-index-<version>.tar.gz` e `index-manifest.json` (con checksum SHA-256):

```bash
bevy-rui-mcp publish-index \
  --rui-path /ruta/donde/clonaste/bevy_rui \
  --out ./dist
```

Note: the `meta.json` inside the tarball is **normalized**: its `rui_path` is emptied before persisting it, so an
absolute path specific to a machine never reaches the distributed data; the bundle identifies the index
only by the `bevy_rui` version, number of records and fingerprint.
### 3. Descargar e instalar un índice precompilado (GitHub Releases o archivo/URL)

Si un usuario no tiene el código fuente de `bevy_rui` localmente o prefiere descargar el índice ya listo:

```bash
# From this repo's GitHub Releases (asset `bevy-rui-index-<version>.tar.gz`, tag `v<version>`)
bevy-rui-mcp fetch-index \
  --repo tu-usuario/bevy_rui_mcp \
  --version 0.1.7

# O desde un archivo .tar.gz local o URL directa
bevy-rui-mcp fetch-index \
  --from ./dist/bevy-rui-index-0.1.7.tar.gz \
  --version 0.1.7
```

Bundles published under an older `cache_version` are also accepted: `fetch-index` installs them and
`status` reports the schema they come from (`index schema : v2 (migrated in memory from v1: ...)`) . A
bump in cache version never orphans an already published bundle.
### 4. Verificar el estado del índice y configuración

```bash
bevy-rui-mcp status --rui-path /ruta/donde/clonaste/bevy_rui
```

`status` prints, among other things: the resolved `rui_path` (or `(none)`, with its source), the resolved
`rui_version` and **where** it came from (`--version`, `BEVY_RUI_VERSION`, config file,
`bevy_rui/Cargo.toml`, `installed precompiled index (<data_dir>/versions)`, ...), whether an index
exists on disk, its schema version (including migrations), the origin of the index
(`precompiled: no `bevy_rui` source tree present or required`), the number of records and the widget catalog.
### 5. Iniciar el servidor MCP (stdio)

```bash
bevy-rui-mcp serve --rui-path /ruta/donde/clonaste/bevy_rui
```
*(Si no se especifica subcomando, `serve` es el modo por defecto. Si aún no existe un índice en `--data-dir` y `--rui-path` es válido, `bevy-rui-mcp` indexa `bevy_rui` automáticamente al arrancar en ~50 ms).*

### 6. Use the MCP server **without** local source of `bevy_rui` (source-free)

Nothing requires a source tree: what travels is the index. A game or software project that depends
on `bevy_rui` only needs the **package name** and the **version**.
on `bevy_rui` only needs the **package name** and the **version**.

```bash
# 1) Get the bundle in a data directory of your choice (GitHub Releases, file or URL)
bevy-rui-mcp fetch-index --from ./dist/bevy-rui-index-0.1.7.tar.gz --data-dir ./my-game/.bevy-rui-mcp-data
# (or: bevy-rui-mcp fetch-index --repo tu-usuario/bevy_rui_mcp --version 0.1.7)

# 2) Run the server with only the data directory: no --rui-path, no --version, no config file
bevy-rui-mcp serve --data-dir ./my-game/.bevy-rui-mcp-data --project-root ./my-game
```

The **version** of `bevy_rui` is derived from the installed index; no configuration file is read, and all the tools
(`rui_widget`, `rui_api`, `rui_search`, `rui_examples`, `rui_theme`, `rui_status`) work with the prebuilt index:
complete guides for widgets, exact signatures, `trait`/`trait_impl` associations, doc links, examples and themes.

## Environment variables (`BEVY_RUI_*`)

| Variable | Description |
| :--- | :--- |
| `BEVY_RUI_MCP_CONFIG` | Explicit path to a `bevy-rui-mcp.config.json` file. |
| `BEVY_RUI_PROJECT_ROOT` | Project root used for auto-detection (default: current directory). |
| `BEVY_RUI_PATH` | Path to a local `bevy_rui` checkout (optional). |
| `BEVY_RUI_VERSION` | Explicit `bevy_rui` version (otherwise resolved from `Cargo.toml`/`Cargo.lock`, the local checkout's manifest, or the installed precompiled index). |
| `BEVY_RUI_DATA_DIR` | Directory for persisted indices (default: `~/.cache/bevy-rui-mcp/data`). |
| `BEVY_RUI_REPO` | `owner/repo` used by `fetch-index` (default: `prfiredragon/bevy_rui_mcp`). |
| `BEVY_RUI_MAX_RESULTS` | Default limit of search results, clamped to 1..=25 (default: 10). |

## What is tracked in git (and why)

- **Tracked**: `data/` (persisted index: `registry.json`, `versions/<version>/records.ndjson` and
`meta.json`) and `dist/` (`bevy-rui-index-<version>.tar.gz` and `index-manifest.json`). They are the
self-contained artifacts that `fetch-index` downloads and `serve` reads, so that a consumer only
needs the `bevy_rui` name and version: never a source checkout, never a config file, never a path
specific to a developer.
- **Ignored**: `target/`, `bevy-rui-mcp.config.json` (absolute machine-specific paths), and any
`*.tar.gz` outside `dist/`.
- Persisted and shipped `meta.json`/`registry.json` store `rui_path: null` (cleared when writing), so no
path specific to an author is ever shipped. Freshness uses a content fingerprint, not a path.

---

## Configuración en Clientes MCP (Antigravity, Claude Code, Cursor, etc.)

### Opción A: Usando flags CLI (Recomendado)

```json
{
  "mcpServers": {
    "bevy_rui": {
      "command": "/ruta/a/bevy_rui_mcp/target/release/bevy-rui-mcp",
      "args": [
        "serve",
        "--rui-path", "/ruta/a/bevy_rui",
        "--data-dir", "/ruta/a/bevy_rui_mcp/data"
      ]
    }
  }
}
```

### Opción B: Usando variables de entorno

```json
{
  "mcpServers": {
    "bevy_rui": {
      "command": "/ruta/a/bevy_rui_mcp/target/release/bevy-rui-mcp",
      "env": {
        "BEVY_RUI_PATH": "/ruta/a/bevy_rui",
        "BEVY_RUI_DATA_DIR": "/ruta/a/bevy_rui_mcp/data"
      }
    }
  }
}
```

---

## Herramientas MCP (`tools`) disponibles

| Herramienta | Descripción |
| :--- | :--- |
| **`rui_search`** | Búsqueda híbrida (símbolos exactos, tokens camelCase/snake_case y documentación) en todo `bevy_rui`. |
| **`rui_api`** | Inspecciona la definición completa de un símbolo (`struct`, `enum`, `trait`, `function`, `method`), sus campos, variantes, métodos en `RuiBuilderExt`, implementación y ejemplos donde se utiliza. |
| **`rui_widget`** | Complete guide of a widget of `bevy_rui` (`button`, `label`, `image`, `layout`, `window_drag`, ...) or complete listing of the catalog. The number of widgets (24 in `bevy_rui` 0.1.7) always comes from the installed index: never hardcoded. |
| **`rui_examples`** | Lista todos los ejemplos oficiales (`examples/*.rs`), filtra por widget o devuelve el código fuente completo de un ejemplo. |
| **`rui_theme`** | Devuelve la referencia completa de `RuiTheme`/`RuiThemeElement` (colores de fondo, acento, bordes, texto, fuentes, tamaños y overrides con `ThemeTextOverride`, `ThemeSecondaryOverride`, `ThemeBorderOverride`, `ThemeAccentOverride`). |
| **`rui_status`** | Muestra el estado del índice cargado, rutas resueltas (`ruiPath`, `dataDir`), versión de `bevy_rui`, versión de `bevy` objetivo y conteo de símbolos/widgets/ejemplos. |

## Recursos MCP (`resources`) disponibles

- `rui://widgets`: JSON catalog of all widgets indexed in `bevy_rui` (the count comes from the installed index: 24 widgets in `bevy_rui` 0.1.7).
- `rui://widget/{name}`: complete guide for a single widget.
- `rui://symbol/{name}`: JSON record complete of any indexed symbol (short name or qualified path).
- `rui://example/{name}`: complete source of an indexed example.

There is no status resource: status is exposed as the `rui_status` tool and as the `bevy-rui-mcp status` subcommand.
