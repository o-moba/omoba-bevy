"""Local accident prevention for production iOS archives, never an approval token.

Refs are local: the operator must fetch origin/main and tags before a deliberate
release. Ignored signing settings and build outputs do not make source dirty.
"""
from pathlib import Path
import re
import subprocess


def require_production_source(root: Path) -> dict:
    def reject(reason):
        raise ValueError(
            f"Production iOS archive blocked: {reason}. For development select the "
            "Omoba Dev scheme or --contour dev. For a production release, fetch "
            "origin/main and tags, then use a clean main checkout or detached HEAD "
            "at an agreed stable vMAJOR.MINOR.PATCH release commit on origin/main.")

    def git(*args, allowed=(0,)):
        try:
            result = subprocess.run(['git', *args], cwd=root, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        except OSError:
            reject('Git metadata is unavailable')
        if result.returncode not in allowed:
            reject('Git metadata or a required reference is unavailable')
        return result

    if Path(git('rev-parse', '--show-toplevel').stdout.strip()).resolve() != root.resolve():
        reject('the source directory is not the Git checkout root')
    revision = git('rev-parse', '--verify', 'HEAD^{commit}').stdout.strip()
    branch = git('symbolic-ref', '--quiet', '--short', 'HEAD', allowed=(0, 1)).stdout.strip()
    if branch and branch != 'main':
        reject(f'checked out branch is {branch!r}, not main')
    if git('status', '--porcelain', '--untracked-files=all').stdout.strip():
        reject('source has uncommitted tracked changes or non-ignored untracked files')
    tags = sorted(tag for tag in git('tag', '--points-at', 'HEAD').stdout.splitlines()
                  if re.fullmatch(r'v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)', tag))
    if not tags:
        reject('HEAD has no exact stable vMAJOR.MINOR.PATCH release tag')
    git('rev-parse', '--verify', 'refs/remotes/origin/main^{commit}')
    if git('merge-base', '--is-ancestor', revision, 'refs/remotes/origin/main',
           allowed=(0, 1)).returncode:
        reject('HEAD is not reachable from origin/main')
    return {'revision': revision, 'branch': branch or None, 'stable_tags': tags}
