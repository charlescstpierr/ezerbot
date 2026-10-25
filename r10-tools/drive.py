import json
import os
from pathlib import Path
import runpy
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, "/Users/cstpierrecharles/.treehouse/pm/ez-i294/.cursor/skills/verify-ezerbot/scripts")
import browser
import verify_lib

evidence = Path(sys.argv[1]).resolve()
assert os.environ.get("CHROME_DEVTOOLS_AXI_SESSION") == "pm-i294"
run = verify_lib.load_run(evidence)
assert run["session"] == "pm-i294"
state = browser.ensure_headless(run)
assert state["session"] == "pm-i294"
os.environ["CHROME_DEVTOOLS_AXI_AUTO_CONNECT"] = "0"
os.environ.pop("CHROME_DEVTOOLS_AXI_USER_DATA_DIR", None)
os.environ.pop("CHROME_DEVTOOLS_AXI_WS_HEADERS", None)
os.environ["CHROME_DEVTOOLS_AXI_BROWSER_URL"] = "http://127.0.0.1:" + str(state["debug_port"])
(evidence / "browser-harness.json").write_text(json.dumps({"driver":"chrome-devtools-axi", "session":"pm-i294", "chrome_pid":state["chrome_pid"], "identity":state["identity"], "profile":state["profile"], "headless":True}, indent=2))
try:
    runpy.run_path(str(Path(__file__).with_name("proof.py")), run_name="__main__")
finally:
    (evidence / "drive.done").write_text("Drive terminé; lire drive.log et cases.json pour le résultat.\n")
