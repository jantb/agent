.PHONY: build run test check fmt clippy install clean

build:
	cargo build

run:
	cargo run

test:
	cargo test

check: fmt clippy test

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

install: check
	cargo install --path .

clean:
	cargo clean
