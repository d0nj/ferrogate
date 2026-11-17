SHELL := /bin/bash
.DEFAULT_GOAL := build

.PHONY: build dev test lint fmt docker clean

build:
	npm run build
	cargo build --release --bin ferrogate
	@echo "Binary: target/release/ferrogate"

dev:
	trap 'kill 0' INT TERM EXIT; cargo run --bin ferrogate & npm run dev & wait

test:
	cargo test --workspace
	npm run lint
	npm run build

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	npm run lint

fmt:
	cargo fmt --all

docker:
	docker build -t ferrogate:latest .

clean:
	rm -rf apps/*/dist target
