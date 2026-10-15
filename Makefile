.PHONY: build run app install zip icon fmt lint test check clean

build:            ## Debug build
	cargo build

run: app          ## Build the .app and launch it
	open VoiceMeter.app

app:              ## Build VoiceMeter.app (release)
	scripts/bundle.sh

install:          ## Build and copy to /Applications
	scripts/bundle.sh --install

zip:              ## Build a distributable zip in dist/
	scripts/bundle.sh --zip

icon:             ## Regenerate the native app icon (macOS only)
	swift scripts/generate-icon.swift assets

fmt:              ## Format code
	cargo fmt

lint:             ## Clippy with warnings as errors
	cargo clippy --release -- -D warnings

test:             ## Unit tests
	cargo test --release

check:            ## Everything CI runs
	cargo fmt --check
	cargo clippy --release -- -D warnings
	cargo test --release
	cargo build --release

clean:
	cargo clean
	rm -rf VoiceMeter.app dist
