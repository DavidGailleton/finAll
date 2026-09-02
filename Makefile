# Makefile for fin-all.
#
# Thin wrappers around Docker Compose, sqlx-cli and cargo-leptos. Targets that
# invoke sqlx or cargo run inside the dev "app" container (Dockerfile.dev),
# which carries sqlx-cli, clippy and rustfmt and has DATABASE_URL set
# (compose.dev.yaml). Start the dev stack (`make dev`) before running them.

COMPOSE      := docker compose
COMPOSE_DEV  := $(COMPOSE) -f compose.yaml -f compose.dev.yaml
COMPOSE_PROD := $(COMPOSE) -f compose.yaml -f compose.prod.yaml

# Non-interactive exec into the dev app container. Under rootful Docker the
# *_AS_USER variant drops to the caller's uid so bind-mount files (new
# migrations, formatted sources) aren't root-owned. Under rootless Docker the
# container's root already maps to the host user, and an explicit --user with
# the host uid is unmapped, so skip the flag there.
HOST_USER        := $(shell id -u):$(shell id -g)
DOCKER_ROOTLESS  := $(shell docker info -f '{{.SecurityOptions}}' 2>/dev/null | grep -q rootless && echo 1)
DEV_EXEC         := $(COMPOSE_DEV) exec -T app
DEV_EXEC_AS_USER := $(if $(DOCKER_ROOTLESS),$(DEV_EXEC),$(COMPOSE_DEV) exec -T --user $(HOST_USER) app)
SQLX             := $(DEV_EXEC_AS_USER) sqlx

.DEFAULT_GOAL := help
.PHONY: help dev prod down prod-down logs \
        migrate-new migrate migrate-revert migrate-info \
        psql check lint fmt test

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) | sort | \
		awk 'BEGIN {FS = ":.*## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

# --- Docker stacks --------------------------------------------------------

dev: ## Start the dev stack in the foreground (hot reload, :3000)
	$(COMPOSE_DEV) up --build

prod: ## Build and start the production stack detached (Caddy + app + db)
	$(COMPOSE_PROD) up -d --build

down: ## Stop and remove the dev stack
	$(COMPOSE_DEV) down

prod-down: ## Stop and remove the production stack
	$(COMPOSE_PROD) down

logs: ## Follow the dev app logs
	$(COMPOSE_DEV) logs -f --tail=100 app

# --- Migrations ----------------------------------------------------------

migrate-new: ## Create a migration file: make migrate-new name=<name>
	@test -n "$(name)" || { echo "Usage: make migrate-new name=<name>"; exit 1; }
	$(SQLX) migrate add $(name)

migrate: ## Apply pending migrations against the dev database
	$(SQLX) migrate run

migrate-revert: ## Revert the last applied migration on the dev database
	$(SQLX) migrate revert

migrate-info: ## Show migration status for the dev database
	$(SQLX) migrate info

# --- Dev tooling -------------------------------------------------------

psql: ## Open a psql shell on the dev database
	$(COMPOSE_DEV) exec db sh -c 'exec psql -U "$$POSTGRES_USER" -d "$$POSTGRES_DB"'

check: ## Type-check the server (ssr) and browser (hydrate) targets
	$(DEV_EXEC) cargo check --features ssr
	$(DEV_EXEC) cargo check --features hydrate --target wasm32-unknown-unknown

lint: ## Run clippy (ssr + hydrate) and sqlfluff on the migrations
	$(DEV_EXEC) cargo clippy --features ssr
	$(DEV_EXEC) cargo clippy --features hydrate --target wasm32-unknown-unknown
	@if command -v sqlfluff >/dev/null 2>&1; then \
		sqlfluff lint migrations/; \
	else \
		echo "sqlfluff not found on host; skipping SQL lint"; \
	fi

fmt: ## Format the Rust sources with rustfmt
	$(DEV_EXEC_AS_USER) cargo fmt

test: ## Run cargo leptos test
	$(DEV_EXEC) cargo leptos test
