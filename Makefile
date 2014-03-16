RUSTC = rustc
RUSTOPTS = -O
BUILDDIR = build
TESTDIR = $(BUILDDIR)/test
FILE = src/tftp/lib.rs

all: test lib

$(BUILDDIR):
	mkdir -p $@

$(TESTDIR): $(BUILDDIR)
	mkdir -p $@

lib: $(BUILDDIR)
	$(RUSTC) $(RUSTOPTS) --out-dir $(BUILDDIR) $(FILE)

test: $(TESTDIR)
	$(RUSTC) --test -o $(TESTDIR)/test $(FILE)
	RUST_LOG=std::rt::backtrace ./$(TESTDIR)/test

bench: $(TESTDIR)
	$(RUSTC) $(RUSTOPTS) --test -o $(TESTDIR)/bench $(FILE)
	./$(TESTDIR)/bench --bench

clean:
	rm -rf $(BUILDDIR)
