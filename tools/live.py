import json
from pathlib import Path
import subprocess
import sys
root = Path(__file__).resolve().parents[1]
evidence = root / "final"
evidence.mkdir(exist_ok=True)
runner = [sys.executable, str(root / "tools/run-proof.py")]
skill = Path("/Users/cstpierrecharles/.treehouse/pm/ez-i294/.cursor/skills/verify-ezerbot/scripts")
def step(log, *args):
    with (root / log).open("w") as output:
        result = subprocess.run([*runner, *map(str, args)], stdout=output, stderr=subprocess.STDOUT)
    print(log, result.returncode, flush=True)
    result.check_returncode()
try:
    step("launch.log", "bash", skill / "launch.sh", "--evidence", evidence, "--sphere", "pages")
    step("final/doctor-before.log", "bash", skill / "doctor.sh", "--evidence", evidence)
    step("final/drive.log", sys.executable, root / "tools/drive.py", evidence)
    step("final/doctor-after.log", "bash", skill / "doctor.sh", "--evidence", evidence)
    step("final/compare.log", "bash", skill / "compare.sh", "--evidence", evidence)
    run = json.loads((evidence / "run.json").read_text())
    with (evidence / "network-loopback.log").open("w") as output:
        for name in ("wrapper", "fake"):
            result = subprocess.run(["lsof", "-nP", "-a", "-p", str(run[name + "_pid"]), "-iTCP"], capture_output=True, text=True)
            output.write(name + "\n" + result.stdout)
            result.check_returncode()
            assert all("127.0.0.1:" in line for line in result.stdout.splitlines()[1:])
except BaseException:
    step("final/cleanup-failure.log", sys.executable, root / "tools/cleanup-safe.py", "--evidence", evidence)
    raise
finally:
    (root / "live.done").write_text("Lire les logs des étapes pour le résultat.\n")
