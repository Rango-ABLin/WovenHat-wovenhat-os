"""Run WovenHat's live ATA/FAT32 mutation/growth regression in QEMU.

This test uses a disposable FAT32 data disk under target/storage-regression. It waits for the storage serial checkpoint instead of the full boot-suite exit:
`test-memory-qemu.py` already owns full boot validation, while this runner owns
the live IDE/FAT32 mutation seam.
"""
import argparse
import shutil
import subprocess
import sys
import time
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument('--qemu', default=shutil.which('qemu-system-x86_64') or r'C:\Program Files\qemu\qemu-system-x86_64.exe')
    parser.add_argument('--firmware')
    parser.add_argument('--timeout', type=float, default=120.0)
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    qemu = Path(args.qemu)
    firmware = Path(args.firmware) if args.firmware else qemu.parent / 'share' / 'edk2-x86_64-code.fd'
    if not qemu.is_file() or not firmware.is_file():
        parser.error('Set --qemu and --firmware to existing QEMU and OVMF files.')

    image = subprocess.check_output(
        ['cargo', 'run', '--quiet', '--features', 'qemu-test', '--', '--print-image'],
        cwd=root,
        text=True,
    ).strip()

    out = root / 'target' / 'storage-regression'
    out.mkdir(parents=True, exist_ok=True)
    serial = out / 'serial.log'
    serial.write_text('')

    disk = out / 'fat32.img'
    if disk.exists():
        disk.unlink()
    subprocess.check_call(['python', str(root / 'scripts' / 'create-fat32.py'), str(disk)], cwd=root)

    vars_src = root / 'OVMF_VARS.fd'
    vars_dst = out / 'OVMF_VARS.fd'
    shutil.copyfile(vars_src, vars_dst)

    command = [
        str(qemu),
        '-machine', 'pc',
        '-m', '256M',
        '-smp', '2',
        '-display', 'none',
        '-serial', f'file:{serial}',
        '-no-reboot',
        '-no-shutdown',
        '-drive', f'if=pflash,format=raw,readonly=on,file={firmware}',
        '-drive', f'if=pflash,format=raw,file={vars_dst}',
        '-drive', f'if=none,id=wovenhatboot,format=raw,file={image}',
        '-device', 'virtio-blk-pci,drive=wovenhatboot,bootindex=1',
        '-drive', f'if=ide,index=0,format=raw,file={disk}',
    ]

    process = subprocess.Popen(command, cwd=root)
    deadline = time.monotonic() + args.timeout
    last_log = ''
    try:
        while time.monotonic() < deadline:
            if serial.exists():
                last_log = serial.read_text(errors='replace')
                storage_passed = '[STORAGE MUTATION] live FAT32 rename/delete/growth/lifecycle: PASSED' in last_log
                boot_passed = '[BOOT] ALL VALIDATIONS PASSED' in last_log
                storage_failed = '[STORAGE MUTATION] live FAT32 rename/delete/growth/lifecycle: FAILED' in last_log
                boot_failed = 'FAILED' in last_log and not storage_passed
                if storage_passed:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
                    print('QEMU storage mutation/growth/lifecycle suite: PASS')
                    print('Serial log:', serial)
                    return 0
                if storage_failed or boot_failed:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
                    print('QEMU storage mutation/growth/lifecycle suite: FAIL', file=sys.stderr)
                    print('Serial log:', serial, file=sys.stderr)
                    print('\n'.join(line for line in last_log.splitlines() if 'FAILED' in line or 'STORAGE MUTATION' in line), file=sys.stderr)
                    return 1
            if process.poll() is not None:
                break
            time.sleep(0.5)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)

    log = serial.read_text(errors='replace') if serial.exists() else last_log
    if '[STORAGE MUTATION] live FAT32 rename/delete/growth/lifecycle: PASSED' in log:
        print('QEMU storage mutation/growth/lifecycle suite: PASS')
        print('Serial log:', serial)
        return 0
    print('QEMU storage mutation/growth/lifecycle suite timed out or exited before success.', file=sys.stderr)
    print('Serial log:', serial, file=sys.stderr)
    return 1


if __name__ == '__main__':
    raise SystemExit(main())
