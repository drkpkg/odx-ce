#!/usr/bin/env bash
set -euo pipefail

# Script maestro para construir todos los paquetes de instalación

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

# Obtener versión actual
version="$("$repo_root/scripts/release/version.sh")"

echo "=========================================="
echo "Building odx packages - Version: $version"
echo "=========================================="
echo ""

# Crear directorio dist si no existe
dist_dir="$repo_root/dist"
mkdir -p "$dist_dir"

# Verificar que Docker esté disponible
if ! command -v docker &> /dev/null; then
    echo "Error: Docker is required but not installed."
    echo "Please install Docker and try again."
    exit 1
fi

if ! docker info &> /dev/null; then
    echo "Error: Docker daemon is not running."
    echo "Please start Docker and try again."
    exit 1
fi

# Función para mostrar el resultado de cada build
show_result() {
    local platform=$1
    local status=$2
    if [ "$status" -eq 0 ]; then
        echo "✓ $platform: SUCCESS"
    else
        echo "✗ $platform: FAILED"
    fi
}

# Contador de éxitos y fallos
success_count=0
fail_count=0

# 1. Build Arch Linux package
echo "Building Arch Linux package..."
if ./packaging/arch/build-archpkg.sh; then
    show_result "Arch Linux" 0
    ((success_count++))
else
    show_result "Arch Linux" 1
    ((fail_count++))
fi
echo ""

# 2. Build Debian package
echo "Building Debian package..."
if ./packaging/debian/build-deb.sh; then
    show_result "Debian" 0
    ((success_count++))
else
    show_result "Debian" 1
    ((fail_count++))
fi
echo ""

# 3. Build Windows installer
echo "Building Windows installer..."
if ./packaging/windows/build-installer.sh; then
    show_result "Windows" 0
    ((success_count++))
else
    show_result "Windows" 1
    ((fail_count++))
fi
echo ""

# Resumen final
echo "=========================================="
echo "Build Summary"
echo "=========================================="
echo "Version: $version"
echo "Success: $success_count"
echo "Failed:  $fail_count"
echo ""

if [ "$fail_count" -eq 0 ]; then
    echo "✓ All packages built successfully!"
    echo ""
    echo "Packages generated in: $dist_dir"
    echo ""
    echo "Files:"
    ls -lh "$dist_dir"/* 2>/dev/null | awk '{print "  " $9 " (" $5 ")"}' || echo "  (no files found)"
    exit 0
else
    echo "✗ Some builds failed. Check the output above for details."
    exit 1
fi
