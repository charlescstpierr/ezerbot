#!/usr/bin/env python3
"""Read Electron v1 fuse wires without changing bytes or signatures."""
from pathlib import Path
import hashlib
import json
import mmap
import sys

source = Path(sys.argv[1])
marker = b'dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX'
names = ['RunAsNode','EnableCookieEncryption','EnableNodeOptionsEnvironmentVariable',
    'EnableNodeCliInspectArguments','EnableEmbeddedAsarIntegrityValidation',
    'OnlyLoadAppFromAsar','LoadBrowserProcessSpecificV8Snapshot',
    'GrantFileProtocolExtraPrivileges','WasmTrapHandlers']
states = {0x30:'disabled',0x31:'enabled',0x72:'removed',0x90:'inherit'}
with source.open('rb') as handle:
    with mmap.mmap(handle.fileno(), 0, access=mmap.ACCESS_READ) as data:
        position = 0
        wires = []
        while (position := data.find(marker, position)) != -1:
            start = position + len(marker)
            version, count = data[start], data[start+1]
            if version != 1 or start+2+count > len(data):
                raise ValueError('Unsupported or truncated fuse wire')
            wire = data[start+2:start+2+count]
            wires.append({'sentinel_offset':position,'version':version,'wire_length':count,
                'raw_bytes':list(wire),'fuses':{names[i] if i<len(names) else f'UnknownFuse{i}':states.get(b,f'unknown-byte-{b}') for i,b in enumerate(wire)}})
            position += len(marker)
        digest = hashlib.sha256(data).hexdigest()
print(json.dumps({'source':str(source.resolve()),'sha256':digest,'execution':'static-only',
    'format_reference':'https://github.com/electron/fuses/blob/main/src/index.ts',
    'index_reference':'https://github.com/electron/fuses/blob/main/src/config.ts',
    'wires':wires},indent=2))
