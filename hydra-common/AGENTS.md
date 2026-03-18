# hydra-common Guidelines

- API v1 types are the wire contract for servers and clients; changes must be additive. You may add new fields or enum variants with sensible defaults, but do not remove or narrow existing information so older peers remain compatible.
- When these shared types gain new fields that `hydra-server` consumes, mirror them in the domain structs and keep the conversion implementations in sync.
- Prefer mandatory fields over optional ones; do not add `Default` implementations for types that should always be explicitly set.
- When adding new fields to API types, update all downstream consumers including TypeScript types in `hydra-web`.
