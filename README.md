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

Ejemplos típicos:

```bash
odx new my_project -v 18.0
cd my_project
odx run
```