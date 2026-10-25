from pathlib import Path
import platform
import sys

CI_KEYS = ['GITHUB_ACTIONS', 'GITHUB_WORKSPACE', 'RUNNER_ENVIRONMENT', 'RUNNER_OS', 'RUNNER_ARCH', 'RUNNER_TEMP']

def github_actions(root, out, environment, tools):
    assert environment.get('GITHUB_ACTIONS') == 'true'
    assert environment.get('RUNNER_ENVIRONMENT') == 'github-hosted'
    assert environment.get('RUNNER_OS') == 'macOS' and environment.get('RUNNER_ARCH') == 'ARM64'
    assert sys.platform == 'darwin' and platform.machine() == 'arm64'
    workspace = Path(environment['GITHUB_WORKSPACE']).resolve()
    temporary = Path(environment['RUNNER_TEMP']).resolve()
    assert root.resolve() == workspace / 'subject'
    assert tools.resolve() == workspace / 'vehicle/.diagnostics/i557'
    assert out.resolve() == temporary / 'i557'
    return 'github-actions'
