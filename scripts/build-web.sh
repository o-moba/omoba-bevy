#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Reuse the canonical second cache. Never create a worktree-local Cargo cache.
game_git_root="$(git rev-parse --path-format=absolute --git-common-dir)/.."
if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  [[ -d "$game_git_root/target-b" ]] || {
    echo 'Set CARGO_TARGET_DIR to the existing shared A or B cache. A new per-worktree cache is not created.' >&2
    exit 1
  }
  export CARGO_TARGET_DIR="$game_git_root/target-b"
fi
command -v trunk >/dev/null || { echo 'Install the existing Trunk build tool first.' >&2; exit 1; }
rustup target list --installed | rg -q '^wasm32-unknown-unknown$' || {
  echo 'Install the Rust wasm32-unknown-unknown target first.' >&2; exit 1;
}
python3 - <<'PY'
from pathlib import Path
import shutil
assets=Path('browser-client/static/assets')
for name in ['terrain-atlas.png','props-atlas.png']:
 target=assets/'world2d'/name;target.parent.mkdir(parents=True,exist_ok=True)
 shutil.copy2(Path('client/assets/world2d')/name,target)
skins=['mossback-teapot','neon-axolotl-courier','origami-storm-heron','clockwork-turnip-oracle','void-jelly-astronaut','cathedral-moth-bellringer','tidal-typewriter-crab']
for skin in skins:
 target=assets/'sprites'/f'{skin}.png';target.parent.mkdir(parents=True,exist_ok=True)
 shutil.copy2(Path('client/assets/sprites')/f'{skin}.png',target)
(assets/'LICENSE.txt').write_text('OMOBA original 2D sprites and terrain: CC0-1.0.\nSource: https://github.com/o-moba/omoba-bevy\nBrowser practice and common authority: AGPL-3.0-only.\nShared types: MPL-2.0.\nBevy: MIT OR Apache-2.0; third-party licenses remain applicable.\n')
PY
# Some automation shells set NO_COLOR=1, while Trunk's clap parser expects a
# boolean spelling. Select the same behavior explicitly for the child process.
(
  cd browser-client
  env -u NO_COLOR -u RUST_LOG trunk build index.html --release --locked --color never --public-url ./ --dist ../builds/browser-practice
)
python3 - <<'PY'
from pathlib import Path
import hashlib,json,re,subprocess
root=Path('builds/browser-practice')
files=sorted(p for p in root.rglob('*') if p.is_file() and p.name!='BUILD_INFO.json')
version=re.search(r'(?m)^version = "([^"]+)"',Path('Cargo.toml').read_text()).group(1)
info={'entry':'index.html','mode':'local-practice','version':version,
 'commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
 'dirty':bool(subprocess.check_output(['git','status','--porcelain'],text=True).strip()),
 'sha256':{str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in files},
 'bytes':sum(p.stat().st_size for p in files),'networked':False,'ranked':False}
(root/'BUILD_INFO.json').write_text(json.dumps(info,indent=2)+'\n')
print(f'Browser practice: {len(files)} files, {info["bytes"]/1024/1024:.1f} MiB at {root}')
PY
