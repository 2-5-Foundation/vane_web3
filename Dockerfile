# # Runtime stage
# FROM ubuntu:22.04

# # Install runtime dependencies
# RUN apt-get update && \
#     apt-get install -y \
#     libssl-dev \
#     ca-certificates \
#     && rm -rf /var/lib/apt/lists/*

# WORKDIR /app

# # Copy the locally built binary (from project root)
# COPY target/release/vane_web3_app /app/vane_web3_app

# # Create logs directory
# RUN mkdir -p /app/logs

# # Make binary executable
# RUN chmod +x /app/vane_web3_app

# CMD ["./vane_web3_app"]


FROM rust:latest
WORKDIR /app
COPY . .

RUN stat /app/Cargo.lock

RUN apt-get update && \
    apt-get install -y \
    protobuf-compiler \
    clang \
    libclang-dev \
    llvm-dev \
    libssl-dev \
    pkg-config

# Set permissions for db directory
RUN chmod -R 777 db/

# Check files before generation
RUN echo "=== Files before Prisma generation ===" && \
    ls -la db/src/

# Run Prisma commands
RUN cargo run -p prisma migrate dev --name initial_migration --create-only --schema="db/schema.prisma"
RUN cargo run -p prisma generate --schema="db/schema.prisma"

# Check files after generation
RUN echo "=== Files after Prisma generation ===" && \
    ls -la db/src/

RUN cargo build --release

ENTRYPOINT ["./target/release/vane_web3_app"]
CMD []



#FROM rust:latest
#WORKDIR /app
#COPY . .
#
#RUN apt-get update && \
#    apt-get install -y \
#    protobuf-compiler \
#    clang \
#    libclang-dev \
#    llvm-dev \
#    libssl-dev \
#    pkg-config
#
#RUN ls -la
#RUN ls -la db/
#RUN cargo check
#RUN cargo build --release



#ENTRYPOINT ["./target/release/vane_web3_app"]
#CMD []


#FROM --platform=linux/arm64 ubuntu:latest
#WORKDIR /app
#
#COPY vane_web3_app .
#
#RUN apt-get update && \
#    apt-get install -y \
#    libssl-dev \
#    && rm -rf /var/lib/apt/lists/*
#
#ENTRYPOINT ["./vane_web3_app"]
#CMD []