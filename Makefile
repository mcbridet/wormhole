.PHONY: release debug clean

RUSTFLAGS := -C target-feature=-crt-static

release:
	RUSTFLAGS="$(RUSTFLAGS)" cargo build --release
	cp target/release/wormhole .

debug:
	RUSTFLAGS="$(RUSTFLAGS)" cargo build
	cp target/debug/wormhole .

clean:
	cargo clean
	rm -f wormhole
