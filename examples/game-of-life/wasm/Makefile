CLANG = /opt/homebrew/opt/llvm/bin/clang

# Use wasm-ld shipped with Rust's toolchain
WASM_LD = $(shell rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/gcc-ld/wasm-ld

game.wasm: game.c
	$(CLANG) --target=wasm32 -O2 -nostdlib -c -o game.o $<
	$(WASM_LD) --no-entry --export-all -o $@ game.o
	rm -f game.o

clean:
	rm -f game.wasm game.o

.PHONY: clean
