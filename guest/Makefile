# Guest image build — doc 08 action 11 / doc 07 §2.
# Boring and explicit: pinned sources, defconfig + fragment kernel, BusyBox rootfs.

include versions.env

BUILD := build
OUT   := out
DL    := dl
JOBS  := $(shell nproc)

KVER  := linux-$(KERNEL_VERSION)
BVER  := busybox-$(BUSYBOX_VERSION)

.PHONY: all fetch kernel rootfs manifest clean

all: manifest

# --- fetch (pinned, sha-verified; record=1 records a new pin) -----------------

fetch: $(DL)/$(KVER).tar.xz $(DL)/$(BVER).tar.bz2

define download
$(DL)/$(2):
	@mkdir -p $(DL)
	@if [ -n "$(4)" ]; then \
	  echo "FETCH (pinned): $(2)"; \
	  curl -fL -o $$@ $(1)/$(2); \
	  echo "$(4)  $$@" > $$@.sha; \
	  sha256sum -c $$@.sha || { echo "PIN MISMATCH for $(2) — investigate before anything else"; exit 1; }; \
	elif [ "$(record)" = "1" ]; then \
	  echo "FETCH (unpinned, recording): $(1)/$(2)"; \
	  curl -fL -o $$@ $(1)/$(2); \
	  sha=$$$$(sha256sum $$@ | cut -d' ' -f1); \
	  sed -i "s|^$(3) :=.*|$(3) := $$$$sha|" versions.env; \
	  echo "recorded $(3) := $$$$sha (source: $(1)) — REVIEW THE DOWNLOAD SOURCE, then commit"; \
	else \
	  echo "REFUSED: $(3) is not pinned — run 'make $(5) record=1' to fetch+record, review, commit"; \
	  exit 1; \
	fi
endef

KERNEL_URL_BASE := https://cdn.kernel.org/pub/linux/kernel/v6.x
BUSYBOX_URL_BASE := https://busybox.net/downloads

$(eval $(call download,$(KERNEL_URL_BASE),$(KVER).tar.xz,KERNEL_SHA256,$(KERNEL_SHA256),fetch))
$(eval $(call download,$(BUSYBOX_URL_BASE),$(BVER).tar.bz2,BUSYBOX_SHA256,$(BUSYBOX_SHA256),fetch))

# --- kernel: defconfig + fragment ----------------------------------------------

$(DL)/$(KVER)/.config: $(DL)/$(KVER).tar.xz $(BUILD)/kernel-fragment.config
	tar -C $(DL) -xf $<
	cd $(DL)/$(KVER) && make defconfig && \
	  scripts/kconfig/merge_config.sh -m .config $(CURDIR)/$(BUILD)/kernel-fragment.config
	cd $(DL)/$(KVER) && make olddefconfig
	touch $@

$(OUT)/kernel: $(DL)/$(KVER)/.config
	mkdir -p $(OUT)
	$(MAKE) -C $(DL)/$(KVER) -j$(JOBS) vmlinux
	cp $(DL)/$(KVER)/vmlinux $@
	$(MAKE) -C $(DL)/$(KVER) -j$(JOBS) clean

kernel: $(OUT)/kernel

# --- rootfs: busybox + platform init --------------------------------------------

$(OUT)/busybox: $(DL)/$(BVER).tar.bz2
	tar -C $(DL) -xf $<
	cp -r $(DL)/$(BVER) $(DL)/$(BVER)-build
	$(MAKE) -C $(DL)/$(BVER)-build defconfig
	# static, no NLS — the guest userland is this one binary
	sed -i -e 's/^# CONFIG_STATIC is not set/CONFIG_STATIC=y/' $(DL)/$(BVER)-build/.config
	# tc breaks against modern kernel headers (CBQ removed) — not needed in the guest
	sed -i -e 's/^CONFIG_TC=y/# CONFIG_TC is not set/' $(DL)/$(BVER)-build/.config
	$(MAKE) -C $(DL)/$(BVER)-build -j$(JOBS)
	cp $(DL)/$(BVER)-build/busybox $@

$(OUT)/initramfs.cpio.gz: $(OUT)/busybox $(BUILD)/init.sh
	rm -rf $(OUT)/rootfs
	mkdir -p $(OUT)/rootfs/bin $(OUT)/rootfs/dev $(OUT)/rootfs/proc $(OUT)/rootfs/sys
	cp $(OUT)/busybox $(OUT)/rootfs/bin/busybox
	for a in sh mount umount poweroff ip echo cat dd sleep base64; do \
	  ln -sf busybox $(OUT)/rootfs/bin/$$a; done
	cp $(BUILD)/init.sh $(OUT)/rootfs/init
	chmod +x $(OUT)/rootfs/init
	cd $(OUT)/rootfs && find . | cpio -o -H newc --quiet | gzip -9 > $(CURDIR)/$@

rootfs: $(OUT)/initramfs.cpio.gz

# --- manifest: the digest pin (§7.1 fixture) ------------------------------------

$(OUT)/manifest.txt: $(OUT)/kernel $(OUT)/initramfs.cpio.gz
	@{ \
	  k=$$(sha256sum $(OUT)/kernel | cut -d' ' -f1); \
	  i=$$(sha256sum $(OUT)/initramfs.cpio.gz | cut -d' ' -f1); \
	  c=$$(sha256sum $(CURDIR)/$(BUILD)/init.sh | cut -d' ' -f1); \
	  echo "kernel $(KERNEL_VERSION) sha256:$$k"; \
	  echo "initramfs busybox-$(BUSYBOX_VERSION) sha256:$$i"; \
	  echo "platform_init sha256:$$c"; \
	} > $@
	cat $@

manifest: $(OUT)/manifest.txt

clean:
	rm -rf $(OUT) $(DL)/$(KVER) $(DL)/$(BVER)-build
