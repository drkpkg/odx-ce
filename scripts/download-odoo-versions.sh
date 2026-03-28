#!/bin/bash
# Script to download Odoo versions as ZIP files for faster testing

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "${SCRIPT_DIR}/odoo-versions.sh"
DOWNLOAD_DIR=".testing/odoo-zips"
# Try tags first, then branches if tags don't work
BASE_URL_TAG="https://github.com/odoo/odoo/archive/refs/tags"
BASE_URL_BRANCH="https://github.com/odoo/odoo/archive/refs/heads"

echo "Downloading Odoo versions for testing..."
echo "========================================"

# Create download directory
mkdir -p "$DOWNLOAD_DIR"

for version in "${VERSIONS[@]}"; do
    zip_file="${DOWNLOAD_DIR}/odoo-${version}.zip"
    
    if [ -f "$zip_file" ]; then
        # Check if file is valid (not an HTML error page)
        size=$(stat -f%z "$zip_file" 2>/dev/null || stat -c%s "$zip_file" 2>/dev/null || echo "0")
        if [ "$size" -gt 1000000 ]; then  # More than 1MB, probably valid
            echo "✓ Odoo ${version} already downloaded: ${zip_file} ($(du -h "$zip_file" | cut -f1))"
            continue
        else
            echo "⚠  Existing file for ${version} is too small, re-downloading..."
            rm -f "$zip_file"
        fi
    fi
    
    echo "Downloading Odoo ${version}..."
    
    # Try tag URL first
    url="${BASE_URL_TAG}/${version}.zip"
    echo "  Trying tag URL: ${url}"
    
    success=false
    # Download with curl or wget
    if command -v curl &> /dev/null; then
        if curl -L -f -o "$zip_file" "$url" 2>/dev/null; then
            # Check if downloaded file is valid (not HTML error)
            if head -n 1 "$zip_file" 2>/dev/null | grep -q "PK"; then
                success=true
            fi
        fi
        
        if [ "$success" = false ]; then
            echo "  Tag URL failed, trying branch URL..."
            url="${BASE_URL_BRANCH}/${version}.zip"
            if curl -L -f -o "$zip_file" "$url" 2>/dev/null; then
                if head -n 1 "$zip_file" 2>/dev/null | grep -q "PK"; then
                    success=true
                fi
            fi
        fi
    elif command -v wget &> /dev/null; then
        if wget -q -O "$zip_file" "$url" 2>/dev/null; then
            if head -n 1 "$zip_file" 2>/dev/null | grep -q "PK"; then
                success=true
            fi
        fi
        
        if [ "$success" = false ]; then
            echo "  Tag URL failed, trying branch URL..."
            url="${BASE_URL_BRANCH}/${version}.zip"
            if wget -q -O "$zip_file" "$url" 2>/dev/null; then
                if head -n 1 "$zip_file" 2>/dev/null | grep -q "PK"; then
                    success=true
                fi
            fi
        fi
    else
        echo "✗ Neither curl nor wget found. Please install one of them."
        exit 1
    fi
    
    if [ "$success" = true ] && [ -f "$zip_file" ]; then
        size=$(du -h "$zip_file" | cut -f1)
        file_size=$(stat -f%z "$zip_file" 2>/dev/null || stat -c%s "$zip_file" 2>/dev/null || echo "0")
        if [ "$file_size" -gt 1000000 ]; then
            echo "✓ Downloaded Odoo ${version} (${size})"
        else
            echo "✗ Downloaded file is too small (${size}), may be invalid"
            rm -f "$zip_file"
        fi
    else
        echo "✗ Failed to download Odoo ${version}"
        rm -f "$zip_file"
    fi
done

echo ""
echo "Download complete!"
echo "ZIP files are in: ${DOWNLOAD_DIR}"
echo ""
echo "To verify downloads:"
echo "  ls -lh ${DOWNLOAD_DIR}"
