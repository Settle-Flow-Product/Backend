# 01 · Wallet sign-in

> *"Kết nối ví & ký challenge — chứng minh quyền kiểm soát địa chỉ; không chuyển tài sản."* — S-02

```mermaid
flowchart TB
  A["Connect wallet"] --> B(["Wallet chooser"])
  B --> C["POST /auth/wallet/challenge"]
  C --> D["Store nonce · 32 bytes · TTL 300s<br/>bound to domain + network"]
  D --> E(["Wallet signs a MESSAGE<br/>not a transaction"])
  E --> F["POST /auth/wallet/verify"]
  F --> G["DELETE ... RETURNING<br/>reads and removes in ONE statement"]
  G -->|"no row"| X["AUTHENTICATION_FAILED"]
  G --> H{"expired?"}
  H -->|"yes"| X
  H --> I{"domain matches?"}
  I -->|"no"| X
  I --> J{"network matches?"}
  J -->|"no"| X
  J --> K{"ed25519 verifies?"}
  K -->|"no"| X
  K --> L["Issue HS256 session · 8h"]
```

## Why the take is one statement

`SELECT` then `DELETE` has a window between them, and a captured challenge replayed into that
window authenticates twice. `DELETE … RETURNING` has no window, which is what makes a nonce single
use.

## Why every refusal looks the same

`X` is one node on purpose. Every failure returns the same message after the same delay, so
nothing reveals whether an address is enrolled. The distinguishing reason goes to the log, never to
the client.

Source: `backend/crates/settle-auth/src/challenge.rs`, `backend/crates/settle-db/src/challenges.rs`.
