
.PHONY: all clippy fmt test build banner tags clean fix readme

all: clippy readme

clippy: fmt
	cargo clippy

fmt: test
	cargo fmt

test: build
	RUST_BACKTRACE=full cargo test

build: banner
	cargo build

banner:
	banner "a build!"

tags:
	ctags -R --exclude=target

clean:
	cargo clean

fix:
	cargo fix --allow-dirty

readme:
	cargo readme -o README.md

