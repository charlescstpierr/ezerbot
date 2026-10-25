import os
from pathlib import Path
import runpy
import sys
sys.dont_write_bytecode = True
assert os.environ.get("CHROME_DEVTOOLS_AXI_SESSION") == "pm-i294"
evidence = Path(sys.argv[1]).resolve()
try:
    runpy.run_path(str(Path(__file__).with_name("proof.py")), run_name="__main__")
finally:
    (evidence / "drive.done").write_text("Drive terminé; lire drive.log et cases.json.\n")
