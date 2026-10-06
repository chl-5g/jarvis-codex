#!/bin/zsh
set -euo pipefail

project_dir=${0:A:h:h}
bundle="$project_dir/src-tauri/target/release/bundle/macos/Jarvis Codex.app"
installed_bundle="/Applications/Jarvis Codex.app"

if [[ ! -d "$bundle" ]]; then
  print -u2 "Release bundle not found: $bundle"
  print -u2 "Run npm run build first."
  exit 1
fi

pkill -f "$installed_bundle/Contents/MacOS/jarvis-codex" 2>/dev/null || true
pkill -f "$bundle/Contents/MacOS/jarvis-codex" 2>/dev/null || true
sleep 1
/usr/bin/ditto "$bundle" "$installed_bundle"
/usr/bin/open "$installed_bundle"
print "Installed and launched: $installed_bundle"
