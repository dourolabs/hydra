# metis-common Guidelines

- API v1 types are the wire contract for servers and clients; changes must be additive. You may add new fields or enum variants with sensible defaults, but do not remove or narrow existing information so older peers remain compatible.
- When these shared types gain new fields that `metis-server` consumes, mirror them in the domain structs and keep the conversion implementations in sync.
