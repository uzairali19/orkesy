.DEFAULT_GOAL := help

.PHONY: help build release run demo check test fmt fmt-check lint ci install clean

help:
	@echo "Targets:"
	@echo "  build        cargo build --workspace"
	@echo "  release      cargo build --workspace --release"
	@echo "  run          cargo run -p orkesy-cli"
	@echo "  demo         cargo run -p orkesy-cli -- --engine fake"
	@echo "  check        cargo check --workspace --all-targets"
	@echo "  test         cargo test --workspace --all-targets"
	@echo "  fmt          cargo fmt --all"
	@echo "  fmt-check    cargo fmt --all -- --check"
	@echo "  lint         cargo clippy --workspace --all-targets -- -D warnings"
	@echo "  ci           fmt-check + lint + test  (mirrors GitHub Actions)"
	@echo "  install      cargo install --path orkesy-cli"
	@echo "  clean        cargo clean"

build:
	cargo build --workspace

release:
	cargo build --workspace --release

run:
	cargo run -p orkesy-cli

demo:
	cargo run -p orkesy-cli -- --engine fake

check:
	cargo check --workspace --all-targets

test:
	cargo test --workspace --all-targets

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

ci: fmt-check lint test

install:
	cargo install --path orkesy-cli

clean:
	cargo clean
