################
##### Builder
FROM rust:slim-bookworm AS chef

WORKDIR /usr/src/rumgap

RUN apt-get update && apt-get install -y pkg-config libssl-dev openssl protobuf-compiler && apt-get clean
COPY rust-toolchain.toml .
RUN rustup toolchain install && cargo install cargo-chef --locked

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Dependencies only, keyed on the recipe (Cargo.toml/Cargo.lock) -- a
# source-only change reuses this cached layer and just compiles rumgap.
FROM chef AS builder
COPY --from=planner /usr/src/rumgap/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --bin rumgap && cp target/release/rumgap /usr/local/bin/rumgap

################
##### Runtime
FROM debian:bookworm AS runtime

# Copy application binary from builder image
COPY --from=builder /usr/local/bin/rumgap /usr/local/bin
COPY log4rs.yml /usr/local/bin

RUN apt-get update && apt-get install -y ca-certificates openssl libssl-dev && apt-get clean

ENV HOST=0.0.0.0
ENV PORT=80
ENV DATABASE_URL="postgres://postgres:postgres@postgres/postgres"
ENV MANGA_UPDATE_INTERVAL_MS=600000

EXPOSE $PORT

# Run the application
CMD ["/usr/local/bin/rumgap"]