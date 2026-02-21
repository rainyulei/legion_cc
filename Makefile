# Legion — Build & Package
# Usage:
#   make build    — compile release binaries
#   make pkg      — create macOS .pkg installer
#   make install  — copy binaries to /usr/local/bin
#   make uninstall — remove installed binaries
#   make clean    — remove build artifacts

VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
ARCH := $(shell uname -m)
PKG_NAME := legion-$(VERSION)-$(ARCH).pkg
PKG_ID := com.legion.cli
INSTALL_DIR := /usr/local/bin

BINARIES := legion legion-dispatch legion-check legion-report legion-status legion-stop
RELEASE_DIR := target/release
PKG_ROOT := target/pkg-root

.PHONY: build pkg install uninstall clean

build:
	cargo build --release

pkg: build
	@echo "==> Packaging $(PKG_NAME)"
	@rm -rf $(PKG_ROOT)
	@mkdir -p $(PKG_ROOT)$(INSTALL_DIR)
	@for bin in $(BINARIES); do \
		cp $(RELEASE_DIR)/$$bin $(PKG_ROOT)$(INSTALL_DIR)/; \
	done
	pkgbuild \
		--root $(PKG_ROOT) \
		--identifier $(PKG_ID) \
		--version $(VERSION) \
		--install-location / \
		$(PKG_NAME)
	@rm -rf $(PKG_ROOT)
	@echo "==> Created $(PKG_NAME)"
	@ls -lh $(PKG_NAME)

install: build
	@echo "==> Installing to $(INSTALL_DIR)"
	@for bin in $(BINARIES); do \
		cp $(RELEASE_DIR)/$$bin $(INSTALL_DIR)/; \
	done
	@echo "==> Done"

uninstall:
	@echo "==> Removing legion binaries from $(INSTALL_DIR)"
	@for bin in $(BINARIES); do \
		rm -f $(INSTALL_DIR)/$$bin; \
	done
	@echo "==> Done"

clean:
	cargo clean
	rm -f legion-*.pkg
