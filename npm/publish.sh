#!/usr/bin/env bash
# Publish the npm packages from release tarballs.
# Usage: npm/publish.sh <artifacts-dir> <version>
# <artifacts-dir> holds prune-<os>-<arch>.tar.gz files as built by .github/workflows/release.yml.
set -euo pipefail

artifacts="$1"
version="$2"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"

# release asset name -> npm os/cpu
targets=(
  "prune-darwin-aarch64:darwin:arm64"
  "prune-linux-aarch64:linux:arm64"
  "prune-linux-x86_64:linux:x64"
)

for target in "${targets[@]}"; do
  IFS=: read -r asset os cpu <<<"$target"
  tarball="$(find "$artifacts" -name "$asset.tar.gz" | head -n 1)"
  if [ -z "$tarball" ]; then
    echo "missing $asset.tar.gz in $artifacts" >&2
    exit 1
  fi
  name="prunecode-$os-$cpu"
  dir="$work/$name"
  mkdir -p "$dir/bin"
  tar -xzf "$tarball" -C "$dir/bin"
  chmod +x "$dir/bin/prune"
  cat >"$dir/package.json" <<JSON
{
  "name": "$name",
  "version": "$version",
  "description": "The $os-$cpu binary for prunecode.",
  "license": "MIT",
  "repository": { "type": "git", "url": "git+https://github.com/dlabadini/prune.git" },
  "os": ["$os"],
  "cpu": ["$cpu"],
  "files": ["bin"]
}
JSON
  (cd "$dir" && npm publish --access public)
done

main="$work/prunecode"
cp -R "$here/prunecode" "$main"
node -e '
  const fs = require("fs");
  const [file, version] = process.argv.slice(1);
  const pkg = JSON.parse(fs.readFileSync(file, "utf8"));
  pkg.version = version;
  for (const dep of Object.keys(pkg.optionalDependencies)) pkg.optionalDependencies[dep] = version;
  fs.writeFileSync(file, JSON.stringify(pkg, null, 2) + "\n");
' "$main/package.json" "$version"
(cd "$main" && npm publish --access public)
