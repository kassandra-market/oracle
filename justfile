# Build both SBF program artifacts (oracle + market).
build: build-oracle build-market

# Build the oracle SBF program artifact (target/deploy/kassandra_oracles_program.so).
build-oracle:
    cargo build-sbf --manifest-path programs/oracles/Cargo.toml

# Build the market SBF program artifact (target/deploy/kassandra_markets_program.so).
build-market:
    cargo build-sbf --manifest-path programs/markets/Cargo.toml

# Build first, then run each program's tests (never test a stale .so).
test: build
    cargo test -p kassandra-oracles-program
    cargo test -p kassandra-markets-program
