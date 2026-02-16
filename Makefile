.PHONY: build install clean

build:
	cargo build --release

install: build
	./target/release/amaranthine install

clean:
	cargo clean
