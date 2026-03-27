# Build release binaries
build:
    cargo build --release -p todomocop-cli -p todomocop-mcp

# Run all tests
test:
    cargo test --workspace

# Build + test
check: test build

# Sync GitHub + Linear tasks
sync:
    cargo run --release -p todomocop-cli -- sync

# Sync GitHub only
sync-github:
    cargo run --release -p todomocop-cli -- sync github

# Sync Linear only
sync-linear:
    cargo run --release -p todomocop-cli -- sync linear
