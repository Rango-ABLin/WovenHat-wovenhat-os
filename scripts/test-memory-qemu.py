"""Run the complete boot validation suite without touching user VM disks."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qemu', default=shutil.which('qemu-system-x86_64') or r'C:\Program Files\qemu\qemu-system-x86_64.exe')
    parser.add_argument('--firmware', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    qemu = Path(args.qemu)
    firmware = args.firmware or qemu.parent / 'share' / 'edk2-x86_64-code.fd'
    if not qemu.is_file() or not firmware.is_file():
        parser.error('Set --qemu and --firmware to existing QEMU and OVMF files.')
    env = os.environ.copy()
    # Existing test-only dead-code warnings are unrelated to the runtime build.
    env['RUSTFLAGS'] = env.get('RUSTFLAGS', '') + ' -A dead_code -A unused_imports'
    image = subprocess.check_output(
        ['cargo', 'run', '--quiet', '--features', 'qemu-test', '--', '--print-image'],
        cwd=root, env=env, text=True).strip()
    out = root / 'target' / 'memory-regression'
    out.mkdir(parents=True, exist_ok=True)
    serial = out / 'serial.log'
    serial.write_text('')
    command = [str(qemu), '-machine', 'q35', '-m', '256M', '-smp', '2',
               '-display', 'none', '-serial', f'file:{serial}', '-no-reboot',
               '-device', 'isa-debug-exit,iobase=0xf4,iosize=0x04',
               '-drive', f'if=pflash,format=raw,readonly=on,file={firmware}',
               '-drive', f'if=none,id=boot,format=raw,readonly=on,file={image}',
               '-device', 'virtio-blk-pci,drive=boot,bootindex=1']
    flags = subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0
    try:
        result = subprocess.run(command, cwd=root, capture_output=True, text=True,
                                timeout=60, creationflags=flags)
    except subprocess.TimeoutExpired:
        print('QEMU boot suite timed out. See', serial, file=sys.stderr)
        return 1
    log = serial.read_text(errors='replace')
    if result.returncode != 33 or '[BOOT] ALL VALIDATIONS PASSED' not in log:
        print(log[-10000:], file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        return 1
    print('QEMU memory/boot suite: PASS (exit 33)')
    print('Serial log:', serial)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
