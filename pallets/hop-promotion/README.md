# pallet-hop-promotion

Promotes near-expiry HOP pool data to permanent chain storage on the Polkadot Bulletin Chain.

## Overview

HOP submissions are short-lived by default. This pallet lets near-expiry data be promoted into `pallet-bulletin-transaction-storage` via general (unsigned, fee-less, priority-0) transactions that only land in otherwise-unused blockspace. The submitter's Bulletin allowance is not debited — the trade-off is that promotion is best-effort, not guaranteed.

The authorize closure verifies the user's submit-time signature and the freshness of the submit timestamp, and rejects promotion for accounts whose Bulletin authorization is missing or expired.

License: Apache-2.0
