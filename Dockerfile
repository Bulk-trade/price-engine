# Use Debian Bullseye slim as the base image for building
FROM debian:bullseye-slim as builder

# Install Rust and required dependencies
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Set the working directory in the container
WORKDIR /usr/src/bulk_price_engine

# Copy the entire project
COPY . .

# Build the application
RUN cargo build --release

# Start a new stage for a smaller final image
FROM debian:bullseye-slim

# Install necessary runtime libraries
RUN apt-get update && apt-get install -y \
    libssl1.1 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from the builder stage
COPY --from=builder /usr/src/bulk_price_engine/target/release/bulk_price_engine /usr/local/bin/bulk_price_engine

# Copy the start script
COPY start.sh /usr/local/bin/start.sh

# Make the script executable
RUN chmod +x /usr/local/bin/start.sh

# Set the startup command to run your binary
CMD ["/usr/local/bin/start.sh"]