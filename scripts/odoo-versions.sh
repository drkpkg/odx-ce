#!/bin/bash
# Single source of truth for Odoo versions used by test/download scripts.
# Source from other scripts: source "$(dirname "$0")/odoo-versions.sh"

_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
VERSIONS=("17.0" "18.0" "19.0")
