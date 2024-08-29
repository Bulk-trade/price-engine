# Use the official Rust image as a parent image
FROM rust:1.79 as builder

# Set the working directory in the container
WORKDIR /usr/src/price-engine

# Copy the current directory contents into the container
COPY . .

# Build the application
RUN cargo build --release

# Start a new stage for a smaller final image
FROM debian:buster-slim

# Install OpenSSL - required for the Rust SSL library
RUN apt-get update && apt-get install -y openssl ca-certificates && rm -rf /var/lib/apt/lists/*

# Copy the binary from the builder stage
COPY --from=builder /usr/src/price-engine/target/release/bulk_price_engine /usr/local/bin/bulk_price_engine

# Copy the start script
COPY start.sh /usr/local/bin/start.sh

# Make the script executable
RUN chmod +x /usr/local/bin/start.sh

# Set the startup command to run your binary
CMD ["/usr/local/bin/start.sh"]