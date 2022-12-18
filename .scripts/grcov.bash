#!/bin/bash

export CARGO_INCREMENTAL=0
export RUSTFLAGS="-Ccodegen-units=1 -Copt-level=0 -Clink-dead-code -Coverflow-checks=off -Cpanic=abort"
export RUSTDOCFLAGS="-Cpanic=abort"

cargo build

cargo test

grcov ./target/debug/ -s . -t html --llvm --branch --ignore-not-existing -o ./target/debug/coverage/
