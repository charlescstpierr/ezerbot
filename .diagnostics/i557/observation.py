import json

import schema

class PreparationFailure(Exception):
    def __init__(self, reason):
        self.reason = reason

def category(error):
    name = type(error).__name__
    name = name if name in schema.ERROR_TYPES else 'Other'
    code = getattr(error, 'errno', None)
    return name, code if type(code) is int and 0 <= code <= 4095 else None

def write(path, value, validator):
    validator(value)
    path.write_bytes(schema.encoded(value))

def empty_attempt(label):
    return dict(label=label, wrapper_return_code=None, reason='pending', error_category=None, errno=None,
                entry_present=False, gate_present=False, metadata_present=False, log_present=False,
                done_present=False, runner=None, gate=None)

def inspect_attempt(out, attempt):
    label = attempt['label']
    for name, path in [('entry', out / (label + '-entry.json')), ('gate', out / (label + '-gate.json')),
                       ('metadata', out / (label + '.json')), ('log', out / (label + '.log')), ('done', out / (label + '.done'))]:
        attempt[name + '_present'] = path.is_file() and not path.is_symlink()
    for key, validator in [('runner', schema.run_entry), ('gate', schema.gate_entry)]:
        name = 'entry' if key == 'runner' else 'gate'
        if not attempt[name + '_present']:
            continue
        try:
            data = json.loads((out / (label + '-' + name + '.json')).read_text())
            validator(data)
            if key == 'runner':
                assert data['label'] == label
            attempt[key] = data
        except (ValueError, OSError, AssertionError, TypeError):
            attempt['reason'] = 'entry_invalid' if key == 'runner' else 'gate_invalid'
            return attempt
    if not attempt['entry_present']:
        attempt['reason'] = 'entry_missing'
    elif attempt['runner']['wrapper_return_code'] != attempt['wrapper_return_code']:
        attempt['reason'] = 'entry_incomplete'
    elif attempt['runner']['reason'] != 'completed':
        attempt['reason'] = attempt['runner']['reason']
    elif not attempt['gate_present']:
        attempt['reason'] = 'gate_missing'
    elif attempt['gate']['wrapper_return_code'] != attempt['runner']['child_return_code']:
        attempt['reason'] = 'gate_incomplete'
    elif attempt['gate']['reason'] != 'completed':
        attempt['reason'] = attempt['gate']['reason']
    elif not attempt['metadata_present']:
        attempt['reason'] = 'metadata_missing'
    elif not attempt['log_present'] or not attempt['done_present']:
        attempt['reason'] = 'pieces_missing'
    elif attempt['wrapper_return_code'] not in [0, 101]:
        attempt['reason'] = 'child_nonzero'
    else:
        attempt['reason'] = 'completed'
    return attempt
