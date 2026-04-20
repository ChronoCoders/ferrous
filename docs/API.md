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
- `getmininginfo`
- `mineblocks`
- `getblock`
- `getblockhash`
- `getbestblockhash`
- `getnewaddress`
- `getbalance`
- `getwalletinfo`
- `encryptwallet`
- `importseed`
- `getshamirshares`
- `listunspent`
- `listaddresses`
- `sendtoaddress`
- `sendrawtransaction`
- `generatetoaddress`
- `getnetworkinfo`
- `addnode`
- `getpeerinfo`
- `getconnectioncount`
- `getnetworkhealth`
- `getrecoverystatus`
- `forcereconnect`
- `resetnetwork`
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

## getblockhash

Returns the block hash for a given height.

### Request

`params` is an array with a single integer: height.

```json
{
  "jsonrpc": "2.0",
  "method": "getblockhash",
  "params": [30000],
  "id": 1
}
```

### Response

`result` is a hex string:

```json
{
  "jsonrpc": "2.0",
  "result": "a68819f243db24659316e301cfb038fe4b62b75055cd30e3b958b23c272e1400",
  "id": 1
}
```

## getmininginfo

Returns the current mining/network statistics.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getmininginfo",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "blocks": 104888,
    "chain": "ferrous",
    "difficulty": 0.0030110102769936487,
    "networkhashps": 86214.60445071748
  },
  "id": 1
}
```

## getnewaddress

Returns a new wallet address string.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getnewaddress",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "address": "Frr1ExampleAddressString"
  },
  "id": 1
}
```

## getbalance

Returns the wallet balance.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getbalance",
  "params": [],
  "id": 1
}
```

### Response

`result` is a JSON object:

```json
{
  "jsonrpc": "2.0",
  "result": {
    "balance": 0.0
  },
  "id": 1
}
```

## listunspent

Returns the wallet UTXO set view.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "listunspent",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "utxos": [
      {
        "txid": "f3c0...",
        "vout": 0,
        "amount": 50.0,
        "confirmations": 101,
        "script_pubkey": "76a914..."
      }
    ]
  },
  "id": 1
}
```

## listaddresses

Returns all wallet addresses.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "listaddresses",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "addresses": [
      "Frr1ExampleAddressString"
    ]
  },
  "id": 1
}
```

## sendtoaddress

Creates and broadcasts a transaction paying `amount` to `address`.

### Request

`params` is an array with two elements: destination address string and amount.

```json
{
  "jsonrpc": "2.0",
  "method": "sendtoaddress",
  "params": ["Frr1ExampleAddressString", 1.25],
  "id": 1
}
```

### Response

`result` is a JSON object containing only the transaction ID. The transaction is submitted to the mempool and included in the next mined block.

```json
{
  "jsonrpc": "2.0",
  "result": {
    "txid": "9b1a..."
  },
  "id": 1
}
```

## sendrawtransaction

Decodes a hex-encoded raw transaction, validates its structure, and submits it to the mempool.

### Request

`params` is an array with a single hex string.

```json
{
  "jsonrpc": "2.0",
  "method": "sendrawtransaction",
  "params": ["0100000001..."],
  "id": 1
}
```

### Response

`result` is the transaction ID as a hex string:

```json
{
  "jsonrpc": "2.0",
  "result": "9b1a...",
  "id": 1
}
```

### Errors

- If the hex is invalid: `"Invalid hex"`
- If the transaction fails to decode: `"Failed to decode transaction"`
- If the transaction fails structure checks: `"Invalid transaction: <details>"`
- If the mempool rejects it: `"Mempool rejected: <details>"`

## getwalletinfo

Returns information about the current wallet state.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getwalletinfo",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "encrypted": false,
    "has_seed": true,
    "receive_addresses": 1,
    "change_addresses": 0,
    "balance_sats": 5000000000
  },
  "id": 1
}
```

Fields:

- `encrypted`: Whether the wallet file is encrypted with ChaCha20-Poly1305.
- `has_seed`: Whether a BIP39 seed is present (enabling deterministic key derivation and Shamir backup).
- `receive_addresses`: Number of receive addresses generated so far.
- `change_addresses`: Number of change addresses generated so far.
- `balance_sats`: Wallet balance in satoshis (frsats).

## encryptwallet

Encrypts the wallet file using ChaCha20-Poly1305 with a PBKDF2-HMAC-SHA512 key derivation (210,000 iterations). The node must be restarted after encryption to load the encrypted wallet.

### Request

`params` is an array with a single string: the passphrase.

```json
{
  "jsonrpc": "2.0",
  "method": "encryptwallet",
  "params": ["my-strong-passphrase"],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": "Wallet encrypted. Restart node to load encrypted wallet.",
  "id": 1
}
```

### Errors

- If the wallet is already encrypted: `"Wallet is already encrypted"`

## importseed

Imports a BIP39 mnemonic phrase, rederives all wallet keys, and saves the wallet. Only 24-word (256-bit entropy) mnemonics are supported. BIP39 passphrase is not yet supported.

### Request

`params` is an array: `[mnemonic, passphrase?]`. Passphrase must be omitted or empty.

```json
{
  "jsonrpc": "2.0",
  "method": "importseed",
  "params": ["word1 word2 ... word24"],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "address_count": 1
  },
  "id": 1
}
```

Fields:

- `address_count`: Number of receive addresses derived after import.

### Errors

- If the mnemonic is invalid: `"Invalid mnemonic: <details>"`
- If the entropy is not 32 bytes: `"Expected 32-byte entropy (256-bit mnemonic), got <n>"`
- If a non-empty passphrase is supplied: `"BIP39 passphrase not yet supported. Import using empty passphrase only."`

## getshamirshares

Splits the wallet's BIP39 seed entropy into M-of-N Shamir shares using GF(256) arithmetic. Any M shares can recover the seed; fewer than M shares reveal nothing.

### Request

`params` is an array: `[m, n]` where `2 ≤ m ≤ n ≤ 10`.

```json
{
  "jsonrpc": "2.0",
  "method": "getshamirshares",
  "params": [2, 3],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "shares": [
      {"index": 1, "share": "a3f0..."},
      {"index": 2, "share": "7c12..."},
      {"index": 3, "share": "e841..."}
    ],
    "m": 2,
    "n": 3
  },
  "id": 1
}
```

Fields:

- `shares`: Array of `{index, share}` objects. `index` is 1-based; `share` is hex-encoded.
- `m`: Minimum number of shares required to recover the seed.
- `n`: Total number of shares produced.

### Errors

- If no seed is present: `"No seed to split"`
- If M/N parameters are invalid: `"Shamir split failed: <details>"`

## generatetoaddress

Mines `nblocks` blocks, paying the coinbase reward to `address`.

### Request

`params` is an array: `[nblocks, address]`.

```json
{
  "jsonrpc": "2.0",
  "method": "generatetoaddress",
  "params": [10, "Frr1ExampleAddressString"],
  "id": 1
}
```

### Response

`result` is a `MineBlocksResponse`:

```json
{
  "jsonrpc": "2.0",
  "result": {
    "blocks": ["000000...", "000000..."]
  },
  "id": 1
}
```

## getnetworkinfo

Returns network configuration and runtime info.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getnetworkinfo",
  "params": [],
  "id": 1
}
```

## getconnectioncount

Returns the number of current P2P connections.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getconnectioncount",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": 1,
  "id": 1
}
```

## resetnetwork

Forces a network reset.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "resetnetwork",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "result": "network reset initiated"
  },
  "id": 1
}
```

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

## getpeerinfo

Returns data about each connected network peer.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getpeerinfo",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "count": 1,
    "peers": ["45.77.153.141:8333"]
  },
  "id": 1
}
```

## getrecoverystatus

Returns the current status of the network recovery manager.

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "getrecoverystatus",
  "params": [],
  "id": 1
}
```

### Response

```json
{
  "jsonrpc": "2.0",
  "result": {
    "partition_detected": false,
    "last_block_age": 120,
    "recovery_attempts": 0
  },
  "id": 1
}
```

## forcereconnect

Manually triggers a full network reconnection (disconnects all peers and restarts discovery).

### Request

```json
{
  "jsonrpc": "2.0",
  "method": "forcereconnect",
  "params": [],
  "id": 1
}
```

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
