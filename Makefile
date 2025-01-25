prog :=mmjab

debug ?=

ifdef debug
  release :=
  target :=debug
  extension :=
else
  release :=--release
  target :=release
  extension :=
endif

build:
	cargo build $(release)

install:
	cp target/$(target)/$(prog) ./$(prog)$(extension) && chmod +x ./$(prog)$(extension)

env:
	cp ./.env.example ./.env

all: build install env

rebuild: clean build install env

clean:
	rm ./.env
	rm ./$(prog)$(extension)

help:
	@echo "usage: make $(prog) [debug=1]"