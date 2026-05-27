# pallet-bulletin-utility

A thin wrapper around `pallet-utility`. It re-exposes `batch`, `batch_all` and `force_batch`,
delegating execution to `pallet-utility`, and adds a `feeless_if` to each: a batch is feeless when it
is non-empty and every inner call is itself feeless. This lets a batch of otherwise-feeless calls
(e.g. transaction-storage `store`/`renew`) avoid being charged a fee, which the plain
`pallet-utility` batch would incur.
