# Two stages, with cache mounts rather than the manifest-stub dance.
#
# The build context is the REPOSITORY ROOT, not this directory: nothing here depends on the
# contract crate today, but the compose file sets the context at the root so that a future
# differential test between the Rust rules and the compiled contract does not require rewriting
# the build.

FROM rust:1.97-slim-bookworm AS builder
WORKDIR /app
COPY backend ./backend
WORKDIR /app/backend
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/backend/target \
    cargo build --release --locked --bin settle-backend \
 && cp target/release/settle-backend /usr/local/bin/settle-backend

FROM debian:bookworm-slim AS runtime

# ca-certificates for TLS to Horizon and the RPC. No OpenSSL: sqlx and reqwest both use rustls
# specifically so this image has no system TLS dependency to keep patched.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl libdbus-1-3 \
 && rm -rf /var/lib/apt/lists/*

# The Stellar CLI signs contract invocations. Secrets live in its keystore, so no private key ever
# passes through this process's environment or its logs.
#
# It has to actually BE here. This comment sat above the COPY below for a while describing a binary
# that was never installed, so every on-chain write from this service failed with "the stellar CLI is
# not installed or not on PATH" — once every five seconds, forever, while the interface showed a
# request that had never reached the chain.
#
# Pinned to the same version the repository's scripts use. `TARGETARCH` comes from BuildKit; the
# release artefacts use GNU triples, hence the mapping.
#
# `libdbus-1-3` above is for this binary: it links the OS secure-storage backend, so without it the
# CLI dies on a missing shared library rather than running. The `stellar --version` at the end is not
# decoration — it is what turns that into a failed BUILD instead of a service that starts fine and
# fails on its first write.
ARG STELLAR_CLI_VERSION=27.1.0
ARG TARGETARCH
RUN set -eux; \
    case "${TARGETARCH}" in \
      arm64) triple=aarch64-unknown-linux-gnu ;; \
      amd64) triple=x86_64-unknown-linux-gnu  ;; \
      *) echo "unsupported architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    url="https://github.com/stellar/stellar-cli/releases/download/v${STELLAR_CLI_VERSION}/stellar-cli-${STELLAR_CLI_VERSION}-${triple}.tar.gz"; \
    curl -fsSL "$url" -o /tmp/stellar.tar.gz; \
    tar -xzf /tmp/stellar.tar.gz -C /usr/local/bin stellar; \
    rm /tmp/stellar.tar.gz; \
    stellar --version

COPY --from=builder /usr/local/bin/settle-backend /usr/local/bin/settle-backend

RUN useradd --system --uid 10001 --create-home settle
USER settle

EXPOSE 8080
ENV BIND=0.0.0.0:8080 RUST_LOG=info,sqlx=warn

# Exec form, so PID 1 receives SIGTERM and the graceful shutdown actually runs.
ENTRYPOINT ["/usr/local/bin/settle-backend"]
