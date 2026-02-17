.PHONY: build install clean deploy bench

build:
	cargo build --release

amrq: build
	cc -O2 -o tools/amrq tools/amrq.c -Iinclude target/release/libamaranthine.a

install: build
	./target/release/amaranthine install

deploy: build amrq
	cp target/release/amaranthine ~/.local/bin/
	cp target/release/libamaranthine.dylib ~/.local/lib/
	cp tools/amrq ~/.local/bin/
	codesign -s - -f ~/.local/bin/amaranthine
	codesign -s - -f ~/.local/bin/amrq
	@echo "deployed — run _reload in MCP to hot-swap"

bench: build
	cc -O2 -o tests/bench tests/bench.c -Iinclude -Ltarget/release -lamaranthine
	DYLD_LIBRARY_PATH=target/release ./tests/bench

clean:
	cargo clean
	rm -f tools/amrq tests/bench
