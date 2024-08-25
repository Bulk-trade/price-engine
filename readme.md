# Bulk Price Engine

## Overview

Bulk Price Engine is a Rust-based WebSocket server that provides real-time price information for various tokens. It uses the Jupiter API to fetch token prices and streams this information to connected clients. The API is built with Actix Web and supports multiple concurrent connections for different tokens.

## Features

- Real-time price updates via WebSocket connections
- Support for multiple tokens (solana)
- Automatic price fetching and caching
- Connection-based resource management
- Exponential backoff for API request retries
- Health check endpoint

## Prerequisites

- Rust (latest stable version)
- Cargo (Rust's package manager)

## Setup

1. Clone the repository:
   ```
   git clone https://github.com/Bulk-trade/price-engine
   cd price-engine
   ```

2. Create a `.env` file in the project root and add the following environment variables:
   ```
   JUPITER_API_URL=https://quote-api.jup.ag/v6/quote
   IP_ADDRESS=0.0.0.0
   PORT=8080
   ```

3. Build the project:
   ```
   cargo build
   ```

## Running the Server

To run the server in development mode:

```
cargo run
```

For production, use:

```
cargo run --release
```

## Usage

### WebSocket Connection

Connect to the WebSocket endpoint:

```
ws://<server-address>/ws/price?token=<token-mint>&token_decimal=<decimal-places>
```

- `<server-address>`: The address where the server is running (e.g., `localhost:8080`)
- `<token-mint>`: The mint address of the token you want to get prices for
- `<decimal-places>`: The number of decimal places for the token (optional, defaults to 6)

Example:
```
ws://localhost:8080/ws/price?token=EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v&token_decimal=6
```

### Health Check

To check the health of the API:

```
GET http://<server-address>/health
```

## Development

For development with auto-reloading:

```
cargo watch -x 'run'
```

## Dependencies

Key dependencies include:
- `actix-web`: Web framework
- `actix-web-actors`: WebSocket support
- `tokio`: Asynchronous runtime
- `serde`: Serialization/deserialization
- `reqwest`: HTTP client
- `backoff`: Exponential backoff for retries

For a full list of dependencies, refer to the `Cargo.toml` file.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the Bulk Labs Limited Open Source Attribution License. See the [LICENSE](LICENSE) file in the root directory of this project for the full license text.