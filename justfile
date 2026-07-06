# Build both SBF program artifacts (oracle + market).
build: build-oracle build-market

# Build the oracle SBF program artifact (target/deploy/kassandra_program.so).
build-oracle:
    cargo build-sbf --manifest-path programs/kassandra/Cargo.toml

# Build the market SBF program artifact (target/deploy/kassandra_market_program.so).
build-market:
    cargo build-sbf --manifest-path programs/kassandra-market/Cargo.toml

# Build first, then run each program's tests (never test a stale .so).
test: build
    cargo test -p kassandra-program
    cargo test -p kassandra-market-program
