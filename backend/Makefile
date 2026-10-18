.PHONY: help all build test test-offline fmt lint migrate verify-migrations verify-constraints clean

DATABASE_URL ?= postgres://settle:settle@localhost:5432/settle

help:  ## Print every target with its purpose.
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  %-20s %s\n", $$1, $$2}'

all: fmt lint test build  ## Everything CI runs.

build: ## Compile the workspace.
	cargo build --workspace

test: ## The whole suite. Pure crates need no database.
	cargo test --workspace

test-offline: ## The suite without anything that wants Postgres.
	cargo test --workspace --exclude settle-db

fmt: ## Format. CI checks rather than writes.
	cargo fmt --all

lint: ## Clippy at CI strictness. A warning is a failure.
	cargo clippy --workspace --all-targets -- -D warnings

migrate: ## Apply migrations to $(DATABASE_URL).
	@for f in $$(ls migrations/*.up.sql | sort); do \
		echo "  $$f"; psql "$(DATABASE_URL)" -v ON_ERROR_STOP=1 -q -f $$f || exit 1; \
	done

## A down migration that forgets an enum type breaks the NEXT deploy, not this one, and the
## failure lands on whoever is redeploying under pressure.
verify-migrations: ## Prove every migration reverses. Uses a throwaway container.
	@./scripts/verify_migrations.sh

## A unique index nobody has watched reject something is a comment.
verify-constraints: ## Prove the constraints refuse what they claim to refuse.
	@./scripts/verify_constraints.sh

clean: ## Remove build output.
	cargo clean
