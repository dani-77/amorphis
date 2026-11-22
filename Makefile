PREFIX ?= /usr/local
DESTDIR ?=
CARGO ?= cargo

.PHONY: all install
all:
	$(CARGO) build --locked --release --bins

install: all
	install -Dm755 target/release/amorphis "$(DESTDIR)$(PREFIX)/bin/amorphis"
	install -Dm755 target/release/amorphis-idle "$(DESTDIR)$(PREFIX)/bin/amorphis-idle"
	install -Dm644 examples/x11.toml "$(DESTDIR)$(PREFIX)/share/amorphis/examples/x11.toml"
	install -Dm644 examples/wayland.toml "$(DESTDIR)$(PREFIX)/share/amorphis/examples/wayland.toml"
	install -Dm644 examples/idle.toml "$(DESTDIR)$(PREFIX)/share/amorphis/examples/idle.toml"
