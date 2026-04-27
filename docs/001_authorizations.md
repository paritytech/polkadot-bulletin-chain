# Authorizations design

## Storage Types

Conceptually there will be 2 types of storage:

- **Temporary storage** — happens through the `store` call.
- **Permanent storage** — happens through the `renew` call (this can also be initiated through the auto-renewal flow).

## Allowance Limits

There are 2 limits on allowances:

- A **soft limit** on temporary storage allowance.
- A **hard limit** on permanent storage allowance.

Once the soft limit is crossed, the `store` calls post this for that account will be on lower priority — meant to utilise the block space when available.

## Auto Renewal

There are some details with Auto Renewal that need to be closed — Cisco had some ideas. Karol Kokoszka — FYI.

## Capacity Planning

Track the overall space utilisation for permanent storage and act when it's close to full — either through:

- A referendum to increase the disk space of collators, or
- Spawning another bulletin chain.


## TODO

- summarize number for PoP (64 MiB) and PoP-lite (2 MiB) and how many user we can have when 1.7 TiB for permanent storage.
- summarize all the impl details

## TODO impl

- track when renewed content is expired and return allowance back to the account and decrease bytes_permanent