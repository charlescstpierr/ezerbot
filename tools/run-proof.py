import os
from pathlib import Path
import subprocess
import sys
root = Path(__file__).resolve().parents[1]
axi_home = root / "axi-home"
axi_home.mkdir(exist_ok=True)
env = dict(os.environ)
env.update({"HOME": str(axi_home), "EZERBOT_VERIFY_LOGIN_HOME": "/Users/cstpierrecharles", "CARGO_HOME": "/Users/cstpierrecharles/.cargo", "RUSTUP_HOME": "/Users/cstpierrecharles/.rustup", "TMPDIR": str(root / "tmp"), "CHROME_DEVTOOLS_AXI_SESSION": "pm-i294", "EZERBOT_VERIFY_CARGO": "/Users/cstpierrecharles/.cargo/bin/cargo", "EZERBOT_VERIFY_CARGO_GATE": "/Users/cstpierrecharles/.treehouse/pm/_orch/bin/cargo-gate", "CARGO_GATE_DIR": "/Users/cstpierrecharles/.treehouse/pm/_orch/locks/cargo", "TEST_GATE_DIR": "/Users/cstpierrecharles/.treehouse/pm/_orch/locks/test", "PYTHONDONTWRITEBYTECODE": "1"})
raise SystemExit(subprocess.call(sys.argv[1:], env=env))
