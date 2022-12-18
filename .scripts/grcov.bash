#!/bin/bash

export RUSTC_BOOTSTRAP=1
export CARGO_INCREMENTAL=0
export RUSTFLAGS="-Zprofile -Ccodegen-units=1 -Copt-level=0 -Clink-dead-code -Coverflow-checks=off -Zpanic_abort_tests -Cpanic=abort"export RUSTDOCFLAGS="-Cpanic=abort"

cargo build
cargo test

grcov ./target/debug/ -s . -t lcov --llvm --branch --ignore-not-existing -o ./target/debug/coverage/
genhtml -o ./target/debug/coverage/ --show-details --highlight --ignore-errors source --legend ./target/debug/lcov.info
