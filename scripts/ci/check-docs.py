#!/usr/bin/env python3
"""Validate consistent diagram sources and initial-render branding in the built docs."""
from pathlib import Path
import re

root = Path(__file__).resolve().parents[2]
header = '---\nconfig:\n  look: classic\n  layout: elk\n---\n'
diagrams = 0
for source in (root / 'docs').rglob('*.md'):
    if '.vitepress' in source.parts:
        continue
    for diagram in re.findall(r'```mermaid\n(.*?)```', source.read_text(), flags=re.S):
        diagrams += 1
        if not diagram.startswith(header):
            raise SystemExit(f'{source.relative_to(root)}: diagram must use classic/elk with the site color theme')
output = root / 'docs/.vitepress/dist'
home = (output / 'index.html').read_text()
if 'ayni-brand-mark' not in home or '<svg' not in home:
    raise SystemExit('The homepage must include the inline Ayni mark before hydration')
if 'rel="icon"' not in home or 'favicon.svg' not in home:
    raise SystemExit('The homepage must link the Ayni SVG favicon')
if not (output / 'favicon.svg').is_file():
    raise SystemExit('The favicon must be included in the published assets')
print(f'Documentation branding and {diagrams} Mermaid configurations are valid.')
