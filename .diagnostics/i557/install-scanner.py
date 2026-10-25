import hashlib
import io
from pathlib import Path
import sys
import tarfile
import urllib.request

destination = Path(sys.argv[1])
archive = urllib.request.urlopen('https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/gitleaks_8.30.1_darwin_arm64.tar.gz', timeout=120).read()
assert hashlib.sha256(archive).hexdigest() == 'b40ab0ae55c505963e365f271a8d3846efbc170aa17f2607f13df610a9aeb6a5'
with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as package:
    member = package.getmember('gitleaks')
    assert member.isfile()
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(package.extractfile(member).read())
    destination.chmod(0o700)
