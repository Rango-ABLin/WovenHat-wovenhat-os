$ErrorActionPreference = "Stop"

$img = Get-ChildItem .\target -Recurse -Filter "wovenhat-os-uefi.img" |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1

if (-not $img) {
    throw "wovenhat-os-uefi.img was not found under .\target. Run cargo build first."
}

$dataImage = Join-Path $PSScriptRoot "wovenhat-disk.img"
if (-not (Test-Path -LiteralPath $dataImage)) {
    & python (Join-Path $PSScriptRoot "scripts/create-fat32.py") $dataImage
    if ($LASTEXITCODE -ne 0) { throw "Failed to create the FAT32 data disk." }
}

Write-Host "Booting WovenHat OS 0.7.0 Stage 9: $($img.FullName)"

& "C:\Program Files\qemu\qemu-system-x86_64.exe" `
    -machine pc `
    -m 1024M `
    -smp 2 `
    -drive "if=pflash,format=raw,readonly=on,file=C:\Program Files\qemu\share\edk2-x86_64-code.fd" `
    -drive "if=pflash,format=raw,file=$PWD\OVMF_VARS.fd" `
    -drive "if=none,id=wovenhatboot,format=raw,file=$($img.FullName)" `
    -device virtio-blk-pci,drive=wovenhatboot,bootindex=1 `
    -drive "if=ide,index=0,format=raw,file=$dataImage" `
    -device virtio-net-pci,netdev=net0,disable-modern=on `
    -netdev user,id=net0 `
    -boot menu=on `
    -serial stdio `
    -no-reboot `
    -no-shutdown
