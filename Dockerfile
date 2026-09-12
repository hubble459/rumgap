################
##### Builder
FROM rust:slim-bookworm AS builder

ENV PROJECT=/usr/src/rumgap

WORKDIR ${PROJECT}

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev openssl protobuf-compiler && apt-get clean

COPY . .

# Cache mounts persist cargo's registry/git checkouts and target/ across builds
# independently of Docker's layer cache (keyed by mount id, not by whether
# Cargo.lock's content hash changed) - so bumping one dependency (e.g.
# manga_parser's git rev) only recompiles that crate and whatever depends on
# it, instead of invalidating this whole layer and rebuilding every
# dependency from scratch. The binary is copied out of the mounted target/
# into a normal layer path here, since mount contents don't persist into the
# image itself.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=${PROJECT}/target,sharing=locked \
    cargo build --release && \
    cp target/release/rumgap /usr/local/bin/rumgap

################
##### Runtime
FROM debian:bookworm AS runtime

# Copy application binary from builder image
COPY --from=builder /usr/local/bin/rumgap /usr/local/bin
COPY log4rs.yml /usr/local/bin

RUN apt-get update && apt-get install -y ca-certificates openssl libssl-dev && apt-get clean

ENV HOST 0.0.0.0
ENV PORT 80
ENV DATABASE_URL "postgres://postgres:postgres@postgres/postgres"
ENV MANGA_UPDATE_INTERVAL_MS 600000

EXPOSE $PORT

# Run the application
CMD ["/usr/local/bin/rumgap"]