import os
from pathlib import Path
import sys

import run
import schema

root = Path(os.environ['I557_ROOT'])
out = Path(os.environ['I557_EVIDENCE'])
label = sys.argv[1]
assert label in schema.LABELS
raise SystemExit(run.execute_ci(root, out, label, sys.argv[2:], os.environ, out / (label + '-entry.json')))
