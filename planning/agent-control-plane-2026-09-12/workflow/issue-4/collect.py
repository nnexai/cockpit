"""Collect retained, read-only evidence from this exact disposable run."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

root = Path('/tmp/cwf12')
out = Path(__file__).resolve().parent
operation_id = '2ed2cdcc-aa76-4055-9173-624805b3f9fb'
companion = root / 'companions' / operation_id
worktree = root / 'worktrees/cockpit-issue-4'
manifest = json.loads((companion / 'manifest.json').read_text())
assert manifest['herdr_workspace_id'] == 'w2'
assert manifest['checkout_path'] == str(worktree)
for source, name in [(root / 'cockpit-state' / (operation_id + '.json'), 'setup-operation.json'), (companion / 'manifest.json', 'companion-manifest.json'), (root / 'cockpit-state/comments/1e5d2100-1f1b-4cbe-91e2-bfc518fb518d.json', 'saved-annotations.json'), (companion / 'issues/import-receipt.json', 'manual-import-receipt.json')]:
    shutil.copyfile(source, out / name)
issue = json.loads((companion / 'issues/github-cockpit-4.json').read_text())
proof = {
    'source_url': issue['url'], 'title': issue['title'], 'issue_state': issue['state'],
    'comments': len(issue['comments']), 'body_bytes': len(issue['body'].encode()),
    'download_method': 'manual gh workaround after Cockpit rejected the GitHub URL',
    'worktree_git_file': (worktree / '.git').read_text().strip(),
    'worktrees_after_close': subprocess.check_output(['git','-C',str(root / 'repos/cockpit'),'worktree','list','--porcelain'],text=True),
    'status_after_close': subprocess.check_output(['git','-C',str(worktree),'status','--short'],text=True),
    'worktree_source_commit': subprocess.check_output(['git','-C',str(worktree),'rev-parse','HEAD'],text=True).strip(),
    'context_files': [{'name': p.name, 'bytes': p.stat().st_size, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()} for p in sorted((companion / 'issues').iterdir())],
}
(out / 'filesystem-proof.json').write_text(json.dumps(proof,indent=2) + '\n')
(out / 'trial-changes.diff').write_text(subprocess.check_output(['git','-C',str(worktree),'diff','--no-ext-diff','--unified=0'],text=True))
(out / 'screenshots.json').write_text(json.dumps([{'path':str(p.relative_to(out)), 'captured_unix':p.stat().st_mtime,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in sorted((out / 'screenshots').glob('*.png'))],indent=2) + '\n')
print(json.dumps({'screenshots':len(list((out / 'screenshots').glob('*.png'))),'worktree':str(worktree),'companion':str(companion),'comments':len(issue['comments'])}))
