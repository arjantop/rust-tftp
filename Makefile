RUSTC = rustc
RUSTOPTS = -O
BUILDDIR = build
TESTDIR = $(BUILDDIR)/test
EXDIR = $(BUILDDIR)/examples
FILE = src/tftp/lib.rs

all: test lib

$(BUILDDIR):
	mkdir -p $@

$(TESTDIR): $(BUILDDIR)
	mkdir -p $@

$(EXDIR): $(BUILDDIR)
	mkdir -p $@

lib: $(BUILDDIR)
	$(RUSTC) $(RUSTOPTS) --out-dir $(BUILDDIR) $(FILE)

test: $(TESTDIR)
	$(RUSTC) --test -o $(TESTDIR)/test $(FILE)
	RUST_TEST_TASKS=1 RUST_LOG=std::rt::backtrace ./$(TESTDIR)/test

bench: $(TESTDIR)
	$(RUSTC) $(RUSTOPTS) --test -o $(TESTDIR)/bench $(FILE)
	./$(TESTDIR)/bench --bench

examples: lib $(EXDIR)
	$(RUSTC) $(RUSTOPTS) -L $(BUILDDIR) -o $(EXDIR)/put src/examples/client/put.rs
	$(RUSTC) $(RUSTOPTS) -L $(BUILDDIR) -o $(EXDIR)/get src/examples/client/get.rs

clean:
	rm -rf $(BUILDDIR)
