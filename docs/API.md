# Ferrous Network JSON-RPC API

This document describes the JSON-RPC API exposed by the Ferrous node. The server is implemented in [rpc/server.rs](file:///c:/ferrous/src/rpc/server.rs) and uses the types from [rpc/methods.rs](file:///c:/ferrous/src/rpc/methods.rs).

- Protocol version: JSON-RPC 2.0
- Transport: HTTP
- Default address: `127.0.0.1:8332` (as configured by the example node)

Each request is a JSON object with:

- `jsonrpc`: `"2.0"`
- `method`: RPC method name
- `params`: array of positional parameters
- `id`: client-provided identifier

Responses:

- On success:
  - `{"jsonrpc":"2.0","result":<value>,"id":<id>}`
- On error:
  - `{"jsonrpc":"2.0","error":{"code":<int>,"message":<string>},"id":<id>}`

## Methods

Implemented methods:

- `getblockchaininfo`
- `mineblocks`
- `getblock`
- `getbestblockhash`
- `stop`

Error codes:

- `-32601`: Method not found
- `-32603`: Internal error (includes validation/mining errors)
- `-32700`: Parse error (invalid JSON)

## getblockchaininfo

Returns basic information about the current blockchain state.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getblockchaininfo",
  "params": [],
  "id": 1
}
```

### Response

`result` is a `GetBlockchainInfoResponse`:

```json
{
  "jsonrpc": "2.0",
  "result": {
    "chain": "ferrous",
    "blocks": 123,
    "headers": 123,
    "bestblockhash": "f3c0..."
  },
  "id": 1
}
```

Fields:

- `chain`: String identifier (`"ferrous"`).
- `blocks`: Height of the active tip.
- `headers`: Same as `blocks` (no header-only mode).
- `bestblockhash`: Hex-encoded hash of the tip block header.

## mineblocks

Mines one or more new blocks on top of the current tip and returns their hashes. This method is intended for regtest/testnet usage.

### Request

`params` is an array with a single integer: number of blocks to mine.

```json
{
  "jsonrpc": "2.0",
  "method": "mineblocks",
  "params": [10],
  "id": 1
}
```

### Response

`result` is a `MineBlocksResponse`:

```json
{
  "jsonrpc": "2.0",
  "result": {
    "blocks": [
      "000000...",
      "000000..."
    ]
  },
  "id": 1
}
```

Fields:

- `blocks`: Array of hex-encoded block header hashes, in the order mined.

### Errors

- If `params` is missing or malformed:
  - `code = -32603`, `message = "Invalid params: expected [nblocks]"`
- If `nblocks` is 0 or greater than 1000:
  - `code = -32603`, `message = "nblocks must be between 1 and 1000"`
- Mining failures:
  - `code = -32603`, `message = "Mining failed: <details>"`

## getblock

Returns information about a block by its hash.

### Request

`params` is an array with a single string: block hash in hex.

```json
{
  "jsonrpc": "2.0",
  "method": "getblock",
  "params": ["000000..."],
  "id": 1
}
```

### Response

`result` is a `GetBlockResponse`:

```json
{
  "jsonrpc": "2.0",
  "result": {
    "hash": "000000...",
    "height": 42,
    "version": 1,
    "merkleroot": "4a5e...",
    "time": 1700000000,
    "nonce": 123456,
    "bits": "1d00ffff",
    "tx": [
      "f3c0...",
      "9b1a..."
    ]
  },
  "id": 1
}
```

Fields:

- `hash`: Hex-encoded block header hash.
- `height`: Block height (genesis is 0).
- `version`: Block version.
- `merkleroot`: Hex-encoded Merkle root of `txid`s.
- `time`: Block timestamp (UNIX seconds).
- `nonce`: Block nonce.
- `bits`: Difficulty target in compact hex format.
- `tx`: Array of hex-encoded transaction `txid`s in the block.

### Errors

- If `params` is missing or malformed:
  - `code = -32603`, `message = "Invalid params: expected [blockhash]"`
- If `blockhash` is not valid hex:
  - `code = -32603`, `message = "Invalid hex"`
- If `blockhash` is not 32 bytes:
  - `code = -32603`, `message = "Invalid hash length"`
- If the block is not found:
  - `code = -32603`, `message = "Block not found"`

## getbestblockhash

Returns the hash of the current best block.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getbestblockhash",
  "params": [],
  "id": 1
}
```

### Response

`result` is a hex string:

```json
{
  "jsonrpc": "2.0",
  "result": "000000...",
  "id": 1
}
```

## stop

Requests the server to shut down after responding.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "stop",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": "stopping",
  "id": 1
}
```

The server will exit after sending this response.

## Examples

### PowerShell (mineblocks)

```powershell
$body = @{
    jsonrpc = "2.0"
    method  = "mineblocks"
    params  = @(10)
    id      = 1
} | ConvertTo-Json

Invoke-RestMethod -Uri http://127.0.0.1:8332 -Method Post -Body $body -ContentType "application/json"
```

### Bash (getblockchaininfo)

```bash
curl -X POST http://127.0.0.1:8332 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getblockchaininfo","params":[],"id":1}'
```

