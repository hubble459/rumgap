################
##### Builder
FROM rust:slim-bullseye as builder

ENV PROJECT /usr/src/rumgap

WORKDIR /usr/src

# Create blank project
RUN USER=root cargo new rumgap && cargo new rumgap/entity --lib && cargo new rumgap/migration --lib

# We want dependencies cached, so copy those first.
COPY Cargo.toml Cargo.lock build.rs icon.ico $PROJECT/
COPY entity/Cargo.toml $PROJECT/entity/
COPY migration/Cargo.toml $PROJECT/migration/
COPY proto $PROJECT/proto/
COPY entity/src $PROJECT/entity/src/
COPY migration/src $PROJECT/migration/src/

# Set the working directory
WORKDIR $PROJECT/

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev openssl protobuf-compiler

# This is a dummy build to get the dependencies cached.
RUN cargo build --release

# Now copy in the rest of the sources
COPY src $PROJECT/src/

## Touch main.rs to prevent cached release build
RUN touch $PROJECT/src/main.rs

# This is the actual application build.
RUN cargo build --release

################
##### Runtime
FROM debian:bullseye AS runtime 

# Copy application binary from builder image
COPY --from=builder /usr/src/rumgap/target/release/rumgap /usr/local/bin
COPY log4rs.yml /usr/local/bin

RUN apt-get update && apt-get install -y ca-certificates openssl libssl-dev && rm -rf /var/lib/apt/lists/*

ENV HOST 0.0.0.0
ENV PORT 80
ENV DATABASE_URL "postgres://postgres:postgres@postgres/postgres"
ENV MANGA_UPDATE_INTERVAL_MS 600000

EXPOSE $PORT

# Run the application
CMD ["/usr/local/bin/rumgap"]