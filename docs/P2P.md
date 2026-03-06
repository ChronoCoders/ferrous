# Ferrous P2P Network Protocol

This document specifies the peer-to-peer network protocol used by Ferrous nodes to communicate, propagate blocks, and relay transactions.

## Protocol Basics

- **Transport**: TCP
- **Default Port**: 8333 (Mainnet), 18333 (Testnet), 18444 (Regtest)
- **Message Structure**:
  - `Magic` (4 bytes): Network identifier
  - `Command` (12 bytes): ASCII command name, null-padded
  - `Length` (4 bytes): Payload length
  - `Checksum` (4 bytes): First 4 bytes of SHA256d(payload)
  - `Payload` (Variable): Message data

## Message Types

### Handshake

1.  **version**: Initiates connection. Contains protocol version, services, timestamp, addr_recv, addr_from, nonce, user_agent, start_height.
2.  **verack**: Acknowledges valid version. Connection is established only after both sides send `verack`.

### Inventory & Data

1.  **inv**: Announces knowledge of one or more objects (Block or Tx). Maximum 50,000 items.
2.  **getdata**: Requests specific objects (Block or Tx) seen in an `inv`.
3.  **block**: Transmits a full serialized block.
4.  **tx**: Transmits a full serialized transaction.
5.  **notfound**: Response when requested data is missing.

### Synchronization

1.  **getheaders**: Requests block headers starting from a specific locator hash. Used for fast headers-first sync.
2.  **headers**: Returns up to 2,000 block headers.

### Discovery

1.  **getaddr**: Requests a list of known active peers.
2.  **addr**: Returns a list of known peer addresses (IP/Port/Services).
3.  **ping**: Periodic liveness check. Contains a nonce.
4.  **pong**: Response to ping. Must contain the same nonce.

## Sync Strategy

### Headers-First

1.  Node sends `getheaders` with its current chain tip locator.
2.  Peer responds with `headers` (up to 2,000).
3.  Node validates PoW and links headers to build the block index.
4.  If 2,000 headers received, repeat step 1.
5.  Once headers are synced, Node identifies missing block data and sends `getdata` (INV_BLOCK) to download full blocks.

### Block Relay

1.  Node A mines or receives a valid new block.
2.  Node A sends `inv` (BLOCK) to connected peers.
3.  Peer B sees `inv`, checks if it already has the block.
4.  If not, Peer B sends `getdata`.
5.  Node A responds with `block`.

### Transaction Relay

1.  Node A validates a new transaction (mempool acceptance).
2.  Node A sends `inv` (TX) to peers.
3.  Peer B requests `getdata` if tx is unknown.
4.  Node A sends `tx`.

## Connection Management

### Discovery & Bootstrapping

- **Seed Nodes**: Hardcoded IP addresses used to bootstrap connections on new nodes.
- **Address Exchange**: Nodes periodically request new peers via `getaddr` and share known active peers via `addr`.
- **Address Manager**: Maintains a database of known peers, tracking their reliability (last success, attempt count) to ensure robust connectivity.

### Keepalive & Timeouts

- **Ping/Pong**: Nodes send `ping` messages every 60 seconds to ensure connections remain active.
- **Dead Peer Detection**: If a peer misses 3 consecutive `pong` responses or sends no data for 20 minutes, it is disconnected.
- **Handshake Timeout**: New connections must complete the Version/Verack handshake within 10 seconds or are dropped.
