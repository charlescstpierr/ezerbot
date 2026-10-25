import importlib.util
import os
from pathlib import Path
import sys
sys.dont_write_bytecode = True
if os.environ.get("CHROME_DEVTOOLS_AXI_SESSION") != "pm-i294":
    raise SystemExit("refus: session explicite pm-i294 requise")
source = Path("/Users/cstpierrecharles/.treehouse/pm/ez-i294/.cursor/skills/verify-ezerbot/scripts/cleanup.py")
spec = importlib.util.spec_from_file_location("cleanup_i294", source)
cleanup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cleanup)
if hasattr(cleanup, "VERIFY_AXI_SESSION"):
    cleanup.VERIFY_AXI_SESSION = "pm-i294"
os.environ["EZERBOT_VERIFY_AXI"] = str(Path(__file__).with_name("axi-cleanup-guard"))
cleanup.main()
