"""Build and verify the WovenHat multicore foundation release; fail on any gate."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qemu')
    parser.add_argument('--firmware')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    out = root / 'target' / 'release-validation'
    out.mkdir(parents=True, exist_ok=True)
    report = []

    def run(name, command):
        print(f'[{name}]', flush=True)
        start = time.monotonic()
        flags = subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0
        result = subprocess.run(command, cwd=root, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, creationflags=flags)
        (out / f'{name}.log').write_text(result.stdout, encoding='utf-8')
        print(result.stdout, flush=True)
        report.append(dict(check=name, passed=result.returncode == 0,
                           seconds=round(time.monotonic() - start, 2)))
        (out / 'results.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
        if result.returncode:
            raise SystemExit(result.returncode)

    run('lint-kernel', ['cargo', 'clippy', '-p', 'wovenhat-kernel', '--target', 'x86_64-unknown-none', '--', '-D', 'warnings'])
    run('lint-host', ['cargo', 'clippy', '-p', 'wovenhat-os', '--', '-D', 'warnings'])
    for source in sorted((root / 'tests').glob('*.rs')):
        executable = out / (source.stem + ('.exe' if os.name == 'nt' else ''))
        run(f'compile-{source.stem}', ['rustc', '--edition=2021', '--test', str(source), '-o', str(executable)])
        run(f'test-{source.stem}', [str(executable)])
    options = []
    for key in ('qemu', 'firmware'):
        if getattr(args, key):
            options.extend([f'--{key}', getattr(args, key)])
    for cpus in (1, 2, 4):
        for suite in ('memory', 'storage'):
            run(f'{suite}-{cpus}-debug', [sys.executable, f'scripts/test-{suite}-qemu.py', '--cpus', str(cpus)] + options)
    run('legacy-pic', [sys.executable, 'scripts/test-memory-qemu.py', '--cpus', '1', '--legacy-irq'] + options)
    for suite in ('memory', 'storage'):
        run(f'{suite}-4-release', [sys.executable, f'scripts/test-{suite}-qemu.py', '--cpus', '4', '--release'] + options)
    run('build-release', ['cargo', 'build', '--release'])
    run('shell-smoke', [sys.executable, 'scripts/test-shell-qemu.py'] + options)
    print('All release gates passed. Report:', out / 'results.json')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
