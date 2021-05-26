RUSTFLAGS="-Cprofile-use=/home/josephguhlin/development/sfasta/merged.profdata -Cllvm-args=-pgo-warn-missing-function" cargo build --release
