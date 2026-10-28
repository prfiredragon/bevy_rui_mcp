# bevy_rui_mcp

Servidor MCP (Model Context Protocol) **100% nativo en Rust** diseñado exclusivamente para la librería de UI [`bevy_rui`](https://github.com/seregonwar/bevy_rui).

Analiza directamente el árbol sintáctico (AST con `syn`) de `bevy_rui` —widgets, componentes, traits de extensión (`RuiBuilderExt`, `RuiRootBuilderExt`, `TooltipExt`), temas (`UiTheme`), sistema de foco/navegación y todos los ejemplos (`examples/*.rs`)— y los expone mediante búsqueda híbrida en memoria y herramientas MCP por `stdio`.

---

## Características principales

- **100% Rust nativo**: Binario único (`bevy-rui-mcp`), sin dependencias de Node.js ni Python.
- **Rutas flexibles (No requiere rutas estándar)**: Funciona sin importar dónde estén clonados `bevy_rui` o `bevy_rui_mcp`, o incluso sin el código fuente de `bevy_rui` usando índices precompilados.
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
   - Si existe un directorio hermano `../bevy_rui` o `./bevy_rui`.

El directorio donde se guardan los índices (`data/`) también es configurable mediante:
1. `--data-dir /ruta/a/data`
2. `BEVY_RUI_DATA_DIR=/ruta/a/data`
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

Genera `bevy-rui-index-v<version>.tar.gz` e `index-manifest.json` (con checksum SHA-256):

```bash
bevy-rui-mcp publish-index \
  --rui-path /ruta/donde/clonaste/bevy_rui \
  --out ./dist
```

### 3. Descargar e instalar un índice precompilado (GitHub Releases o archivo/URL)

Si un usuario no tiene el código fuente de `bevy_rui` localmente o prefiere descargar el índice ya listo:

```bash
# Desde un repositorio de GitHub Releases (usa el tag index-v<version>)
bevy-rui-mcp fetch-index \
  --repo tu-usuario/bevy_rui_mcp \
  --version 0.1.7

# O desde un archivo .tar.gz local o URL directa
bevy-rui-mcp fetch-index \
  --from ./dist/bevy-rui-index-v0.1.7.tar.gz \
  --version 0.1.7
```

### 4. Verificar el estado del índice y configuración

```bash
bevy-rui-mcp status --rui-path /ruta/donde/clonaste/bevy_rui
```

### 5. Iniciar el servidor MCP (stdio)

```bash
bevy-rui-mcp serve --rui-path /ruta/donde/clonaste/bevy_rui
```
*(Si no se especifica subcomando, `serve` es el modo por defecto. Si aún no existe un índice en `--data-dir` y `--rui-path` es válido, `bevy-rui-mcp` indexa `bevy_rui` automáticamente al arrancar en ~50 ms).*

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
| **`rui_widget`** | Guía completa de un widget de `bevy_rui` (ej. `button`, `slider`, `tabs`, `docks`, `windows`, `file_dialog`, `code_editor`, `color_picker`, `menu`, `textbox`, `dropdown`, `scrollview`, `viewport`, `accordion`, `canvas_layer`) o listado del catálogo completo de widgets. |
| **`rui_examples`** | Lista todos los ejemplos oficiales (`examples/*.rs`), filtra por widget o devuelve el código fuente completo de un ejemplo. |
| **`rui_theme`** | Devuelve la referencia completa de `UiTheme` (colores de fondo, acento, bordes, texto, fuentes, tamaños y overrides con `ThemeTextOverride`, `ThemeSecondaryOverride`, `ThemeBorderOverride`, `ThemeAccentOverride`). |
| **`rui_status`** | Muestra el estado del índice cargado, rutas resueltas (`ruiPath`, `dataDir`), versión de `bevy_rui`, versión de `bevy` objetivo y conteo de símbolos/widgets/ejemplos. |

## Recursos MCP (`resources`) disponibles

- `bevy-rui://status`: Resumen en JSON del índice cargado y rutas activas.
- `bevy-rui://widgets`: Catálogo en JSON de todos los widgets indexados en `bevy_rui`.
- `bevy-rui://symbol/{qualified_path}`: Registro completo en JSON de cualquier símbolo indexado.
