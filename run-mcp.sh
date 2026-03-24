#!/bin/sh
export GITHUB_TOKEN=$(gh auth token 2>/dev/null)
export LINEAR_API_KEY=$(cat /run/secrets/linear_api_key 2>/dev/null)
exec "$(dirname "$0")/target/release/todomocop-mcp" "$@"
