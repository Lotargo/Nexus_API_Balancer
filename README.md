<div align="center">

# Nexus API Balancer

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![OAuth 2.1](https://img.shields.io/badge/Security-OAuth_2.1-green.svg)](https://oauth.net/2.1/)
[![MCP](https://img.shields.io/badge/Protocol-MCP-green.svg)](https://modelcontextprotocol.io/)
[![Scalar](https://img.shields.io/badge/Docs-Scalar-green.svg)](https://scalar.com/)
[![SQLx](https://img.shields.io/badge/Database-SQLx-green.svg)](https://github.com/launchbadge/sqlx)

**A high-performance, asynchronous API load balancer designed for decentralized AI networks.**  
_Secure, Scalable, and MCP-ready._

</div>

---

## 🚀 Features

- **High Concurrency**: Asynchronous pool management using `tokio` and `async-channel`.
- **Persistent Storage**: SQLite integration for tracking pools, keys, and clients with full transactionality.
- **Observability**: Detailed request logging for analytics, latency tracking, and error auditing.
- **Admin Protection**: Dedicated administrative layer secured via `.env` secrets and `X-Admin-Key` headers.
- **Client Isolation**: Strict partitioning ensures clients only see and access their assigned pools.
- **Transparent Proxy**: Automatic header injection for OpenAI, Anthropic, and Google Gemini providers.
- **OAuth 2.1 & Hybrid Auth**: Mandatory token validation with Master Key and Public Registration options.
- **Dynamic Key Management**: Export/Import provider keys via API with zero-downtime persistence.
- **MCP Enabled**: Integrated Model Context Protocol server with advanced key management tools.
- **Interactive Documentation**: Premium API explorer via **Scalar** available at `/scalar`.
- **Graceful Shutdown**: Proper signal handling (Ctrl+C) for clean termination and resource cleanup.
- **Dynamic Configuration**: Hot-reloading of configuration via `ArcSwap` and secure API.

---

## 🏗 Architecture

```mermaid
graph TD
    Client[AI Client / Swarm Node] -- OAuth 2.1 Bearer --> API[Axum REST/MCP API]
    Admin[Administrator] -- X-Admin-Key --> API
    API -- Extract Claims --> Auth[Auth Manager]
    Auth -- Validate JWT / Admin Secret --> API
    API -- Isolation Filter --> DB[(SQLite DB)]
    API -- Acquire Slot --> Pool[Key Pool]
    Pool -- Shared State --> Key[API Key Inner]
    Key -- Check RPS/TTL/Ban --> Pool
    Pool -- Slot Granted --> Worker[Worker Thread]
    Worker -- Execute Request --> Provider[External API Provider]
    Worker -- Release Slot --> Pool
    API -- Async Log --> DB
```

---

## 📊 Performance Benchmarks

The following metrics were captured under a stress load of **500 concurrent requests** targeting a pool with a combined limit of **17 RPS**.

### Core Metrics Summary

| Metric               | Performance                  | Status              |
| :------------------- | :--------------------------- | :------------------ |
| **Throughput**       | 1,651.56 req/sec             | ⚡ High Performance |
| **Request Accuracy** | 100% (Exactly 17 authorized) | 🎯 Precision        |
| **Avg Latency**      | 2.86 ms                      | 🚀 Low Latency      |
| **P95 Latency**      | 8.00 ms                      | 📉 Stable           |
| **Error Rate**       | 0.0%                         | 🛡️ Reliable         |

### Visual Analysis

#### 1. Latency Distribution

The chart below illustrates the distribution of response times under load. The majority of requests are handled within the 2-5ms range, confirming the minimal overhead introduced by the balancing and OAuth 2.1 validation layers.

![Latency Distribution](assets/latency_distribution.png)

#### 2. Rate Limiting Effectiveness

During stress testing (500 concurrent requests), the Nexus Balancer demonstrates precise control over downstream resource consumption. It strictly enforces the configured global limits (17 RPS in this test), shielding external providers from potential flooding.

![Request Outcomes](assets/request_outcomes.png)

---

## 🛠 Quick Start

### 1. Prerequisites

- Rust 1.75 or higher
- Cargo

### 2. Installation

```bash
git clone https://github.com/nexus/nexus-balancer.git
cd nexus-balancer
cargo build --release
```

### 3. Configuration

1. Copy the example files:
   ```bash
   cp .env.example .env
   cp config.yaml.example config.yaml
   ```
2. Edit `.env` and `config.yaml` to suit your needs. See `config.yaml.example` for all available parameters.

### 4. Running the Server

```bash
cargo run
```

_The server will display a professional ASCII banner and provide links to the API and documentation._

### 5. Client Registration (Admin)

To register a new client and generate an API key (JWT):

```bash
curl -X POST http://127.0.0.1:8080/admin/clients \
     -H "X-Admin-Key: your-admin-secret" \
     -H "Content-Type: application/json" \
     -d '{"id": "my_client", "name": "Team A"}'
```

### 6. Local Usage (No Auth)

For local or private usage, you can disable authentication in `config.yaml`. In this mode, all requests are automatically granted **Admin** privileges.

### 7. Interactive Testing

Once the server is running, visit:

- **Scalar UI**: `http://127.0.0.1:8080/scalar` (Interactive API explorer)
- **Stats**: `http://127.0.0.1:8080/stats` (Requires `X-Admin-Key`)

---

## 📡 API Reference

### Operational Endpoints

| Method  | Endpoint   | Description                      | Auth Required      |
| :------ | :--------- | :------------------------------- | :----------------- |
| `POST`  | `/execute` | Run a task through the balancer  | OAuth 2.1 (Client) |
| `ANY`   | `/proxy/:pool/*path` | Transparent provider proxy | OAuth 2.1 / Master |
| `POST`  | `/auth/register` | Self-service client registration | Public (Optional) |
| `GET`   | `/stats`   | View real-time DB analytics      | Admin Key          |
| `GET`   | `/config`  | View current configuration       | Admin Key          |
| `PATCH` | `/config`  | Update configuration dynamically | Admin Key          |
| `GET`   | `/admin/keys/:pool/:id` | Export key with secret | Admin Key |
| `POST`  | `/admin/keys/:pool` | Import new key dynamically | Admin Key |
| `POST`  | `/mcp`     | MCP JSON-RPC interface           | OAuth 2.1 / Admin  |

### MCP (Model Context Protocol)

The balancer exposes an MCP-compliant endpoint at `/mcp` (JSON-RPC 2.0).

**Tools:**

- `list_pools`: Returns a list of pools with their descriptions.
- `update_description`: Allows an agent to update a pool's description.
- `export_key`: Retrieve a key's secret by its ID (Admin only).
- `import_key`: Dynamically add a new key to a running pool (Admin only).

---

## 🧪 Testing (E2E)

The project uses a unified, high-performance E2E testing and benchmarking suite written in Rust.

### Running the Rust E2E Suite
1. Ensure the Nexus Balancer is running (`cargo run`).
2. Navigate to the test directory and run the suite:
   ```bash
   cd tests/nexus_e2e
   cargo run
   ```

**The suite automatically performs:**
- **Concurrency Test**: 30 simultaneous requests with pool isolation verification.
- **Dynamic Key Test**: Real-time export, import, and rotation validation.
- **MCP Validation**: JSON-RPC toolchain and resource checking.
- **Mock Provider**: Includes an internal high-speed mock server.

---

## 📜 License

Distributed under the **Apache License 2.0**. See `LICENSE` for more information.
