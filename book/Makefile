default: install

all: hooks install build


h help:
	@grep '^[a-z]' Makefile


.PHONY: hooks
hooks:
	cd .git/hooks && ln -s -f ../../hooks/pre-push pre-push

install:
	cargo install mdbook mdbook-mermaid mdbook-toc mdbook-admonish


s serve:
	mdbook serve


build:
	mdbook build