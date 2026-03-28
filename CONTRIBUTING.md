# Guía de contribución

Gracias por interesarte en **odx**. Este documento resume cómo preparar el entorno, probar cambios y enviar contribuciones.

## Alcance del proyecto

odx CE es una CLI en Rust para proyectos Odoo. La edición community usa el núcleo **vanilla** de Odoo (sin parches al repositorio oficial). Las contribuciones deben mantener ese alcance: no reintroducir flujos de parcheo del core salvo que el proyecto cambie explícitamente de política.

## Requisitos

- [Rust](https://rustup.rs/) (toolchain **stable**, coherente con CI).
- **Git** (la CLI y los tests de integración lo usan al crear proyectos con `odx new`).
- **Python** 3.10+ si vas a probar flujos reales de Odoo fuera de los tests automatizados ligeros.

## Desarrollo local

```bash
cargo build
cargo run -- --help
```

Formateo y comprobaciones habituales (recomendado antes de abrir un PR):

```bash
cargo fmt
cargo clippy -- -D warnings
```

## Tests

- **CI** (`.github/workflows/ci.yml`) ejecuta `cargo build` y `cargo test --lib`.
- **Suite completa** (incluye `tests/integration_tests.rs`): clona Odoo desde GitHub y puede tardar y requerir red.

```bash
cargo test
```

Si solo quieres validar la biblioteca como en CI:

```bash
cargo test --lib
```

## Pull requests

1. Abre un PR con una descripción clara del **qué** y el **por qué**.
2. Prefiere cambios **acotados** a un objetivo; evita refactors masivos mezclados con la funcionalidad.
3. Mantén el estilo del código existente (nombres, organización de módulos, mensajes de error al usuario).
4. Si el cambio afecta comportamiento visible para usuarios, menciónalo en el cuerpo del PR.

## Commits (convención opcional)

Si quieres alinear con el historial del proyecto, puedes usar un prefijo en la primera línea del mensaje:

- `[FEAT]` nueva funcionalidad
- `[FIX]` corrección
- `[BUG]` fallo reproducible corregido
- `[ADD]` adición de archivos o piezas
- `[MIG]` migración o cambio de compatibilidad

La primera línea debe ser un resumen breve en inglés; el cuerpo puede detallar en el idioma que prefieras.

## Empaquetado y releases

Los mantenedores generan artefactos con los scripts en `packaging/` y `scripts/release/`. No hace falta que cada contribución construya paquetes `.deb`/Windows/Arch; basta con que `cargo build` y los tests que apliquen pasen en tu entorno.

## Dudas

Si algo no está cubierto aquí, abre un issue en el repositorio o pregunta en el PR para alinear el enfoque antes de invertir mucho tiempo en un cambio grande.
