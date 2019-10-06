MODE ?= release
EFI := target/x86_64-unknown-uefi/$(MODE)/judge-duck.efi
OVMF := OVMF.fd
ESP := esp
QEMU_ARGS := -nographic
#	-debugcon file:debug.log -global isa-debugcon.iobase=0x402


ifeq (${MODE}, release)
	BUILD_ARGS += --release
endif

.PHONY: build run header asm

build:
	cargo xbuild --target x86_64-unknown-uefi $(BUILD_ARGS)

run: build
	mkdir -p $(ESP)/EFI/Boot
	cp $(EFI) $(ESP)/EFI/Boot/BootX64.efi
	qemu-system-x86_64 \
		-bios ${OVMF} \
		-drive format=raw,file=fat:rw:${ESP} \
		$(QEMU_ARGS)

header:
	cargo objdump -- -h $(EFI) | less

asm:
	cargo objdump -- -d $(EFI) | less
