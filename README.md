# odx

<div align="center">
  <img src="docs/images/logo.svg" alt="odx logo" width="200" height="200">
</div>

## Descripción del Proyecto

**odx** es una CLI para crear y operar proyectos de desarrollo con Odoo.

### Dependencias del sistema

- `python` (venv + pip)
- `docker` / `docker compose` (opcional, para PostgreSQL)
- `psql` (opcional, para utilidades de DB / cleanup)

## Configuración e Instalación

1. Build:

```bash
cargo build
```

2. Ejecutar:

```bash
./target/debug/odx --help
```

### Construir paquetes localmente

**Opción 1: Construir todos los paquetes**

```bash
./scripts/release/build-all.sh
```

**Opción 2: Construir paquetes individualmente**

Los scripts dejan los artefactos en `dist/`:

```bash
./packaging/arch/build-archpkg.sh      # Arch Linux
./packaging/debian/build-deb.sh        # Debian
./packaging/windows/build-installer.sh # Windows
```

## Uso del Proyecto

### Comandos

- `odx run`
- `odx update -d <database>`
- `odx update-module <module> -d <database>`
- `odx shell -d <database>`
- `odx db start|stop|logs|ls|psql`
- `odx db drop <database> [--force] [--if-exists]`
- `odx i18n -d <database> [-m <module>] [--lang <code>]`
- `odx test [<tags>...]`
- `odx install`
- `odx sync`
- `odx store ls|path|add <version>|rm <version>`
- `odx clean`
- `odx new <project> -v <version> [--cd]`
- `odx doctor`

Opción global: `--python <version>` (por ejemplo `3.11`).

### Fuente de Odoo compartida

El código de Odoo **no** se copia dentro de cada proyecto: odx mantiene una copia por
versión en `~/.cache/odx/odoo/<version>` y todos los proyectos apuntan ahí. Un proyecto
nuevo pesa kilobytes en vez de ~1.2 GB, y crear el segundo proyecto de una versión ya
descargada no usa red.

- `.odx.toml` en el proyecto fija la versión (`[odoo] version = "18.0"`).
- `odx store ls` lista las versiones; `odx store path` imprime la ruta en uso.
- `odx install` descarga la versión del proyecto si falta (útil tras clonar el repo).
- `odx sync` actualiza esa copia compartida: afecta a todos los proyectos de esa versión.
- `ODX_ODOO_STORE` cambia la ubicación del store; `ODX_ODOO_PATH` o `[odoo] path` en
  `.odx.toml` fuerzan un checkout concreto.
- Los proyectos antiguos con `src/odoo` siguen funcionando tal cual.

### Depuración (DAP)

`odx run`, `odx test` y `odx shell` inician Odoo siempre con un listener DAP
(debugpy) en `127.0.0.1:5678`: no hace falta ninguna opción para activarlo. Se
conecta cualquier cliente DAP (VS Code/Cursor, nvim-dap); `odx new` genera
`.vscode/launch.json` con la configuración de attach.

- `--debug-port <N>`: cambia el puerto (si está ocupado, odx usa el siguiente libre).
- `--debug-wait`: no arranca hasta que un cliente se conecte (`run` y `test`).
- `breakpoint()` en el código del addon rompe en el cliente conectado; sin cliente
  conectado no hace nada.

Ejemplos típicos:

```bash
odx new my_project -v 18.0
cd my_project
odx run
```