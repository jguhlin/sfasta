#!/bin/bash

export CARGO_INCREMENTAL=0
export RUSTFLAGS="-Cinstrument-coverage"
export RUSTDOCFLAGS="-Cpanic=abort"

cargo build
cargo test

grcov ./target/debug/ -s . -t html --llvm --branch --ignore-not-existing -o ./target/debug/coverage/
