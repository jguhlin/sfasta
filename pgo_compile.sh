RUSTFLAGS="-Cprofile-use=/home/josephguhlin/development/sfasta/merged.profdata -Cllvm-args=-pgo-warn-missing-function" cargo build --release

RUSTFLAGS="-Cprofile-use=/home/josephguhlin/development/sfasta/merged.profdata -Cllvm-args=-pgo-warn-missing-function" cargo build --release --target=x86_64-unknown-linux-gnu
