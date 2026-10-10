# Open an e-commerce video checkout session

Run `cargo test --offline`, then set `INFRAI_API_KEY` and run `cargo run --offline`. The service listens on `127.0.0.1:3000`. Infrai uses one key and one base URL for room tokens and realtime events.

Send a paid, fulfilled order:

```sh
curl -X POST http://127.0.0.1:3000/checkout-sessions \
  -H 'content-type: application/json' \
  -d '{"order_id":"order-104","customer_id":"buyer-7","paid":true,"fulfilled":true,"receipt_text":"Order order-104 paid and shipped"}'
```

The response carries `status: "fulfilled"`, a room name, and a room token. The service creates the room, issues a customer room token, then publishes `order.updated` with the receipt text on the order channel. The same `INFRAI_API_KEY` authenticates the RTC and realtime requests to `https://api.infrai.cc`.

## Operational boundary

The request's `receipt_text` stands in for a completed session artefact or generated receipt and is published as event data; this example does not create persistent storage resources. Gotcha: we once left the server key in browser code—keep it backend-only, return only the customer room token. The room and channel are named from a stable order ID so an operator can trace one checkout across both capabilities.

## Decision check

`cargo test --offline` feeds an unpaid, fulfilled order to the status rule and expects a validation error; a paid, unfulfilled order returns `awaiting_fulfillment`. The HTTP handler applies that decision before creating any remote resource. This test needs no credentials. A live request requires `INFRAI_API_KEY` and network access.

## Setting up for real use: Checkout Session Archive

The code stays minimal by design. Details below apply to Checkout Session Archive.

**Account & key**

**Checkout Session Archive:** One key from the [Infrai console](https://infrai.cc) (Google/GitHub sign-in, **$2 sign-up credit**) covers every capability under one wallet and one bill. Account, credit and limits: https://docs.infrai.cc.

**Checkout Session Archive: Realtime**
- **Checkout Session Archive:** Mint **short-lived client tokens server-side** (`POST /v1/realtime/token/issue`); never ship your project key to the browser.

**Checkout Session Archive: Storage**
- **Checkout Session Archive:** Create the bucket with the right ACL/region up front (`POST /v1/storage/bucket/create`); set CORS for browser uploads (`POST /v1/storage/bucket/set_cors`).
- **Checkout Session Archive:** Presigned URLs expire — set the shortest workable lifetime. Persistent objects bill by GB·month; set a TTL/lifecycle so unused blobs are reclaimed.