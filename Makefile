prog :=mmjab

debug ?=

$(info debug is $(debug))

ifdef debug
  release :=
  target :=debug
  extension :=-debug
else
  release :=--release
  target :=release
  extension :=
endif

build:
	cargo build $(release)

install:
	cp target/$(target)/$(prog) ./$(prog)$(extension) && chmod +x ./$(prog)$(extension)

all: build install

clean:
	rm ./$(prog)$(extension)

help:
	@echo "usage: make $(prog) [debug=1]"