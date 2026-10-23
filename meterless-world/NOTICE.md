ExtraWork vendors the Meterless World Model reference implementation.

Source: https://github.com/Meterless/Meterless
Commit: 1742dec56a66e58eef9dda2d2b865823758ca8cd
Path: engines/world-model/reference and engines/world-model/conformance
License: Apache License, Version 2.0 (see vendor/LICENSE)

Those vendor files are unmodified. ExtraWork's adapter is src/project.ts and src/server.ts.
The reference is not the system of record for money. ExtraWork keeps durable case
records in PostgreSQL and projects stable external identities into the World Model.
Financial and document identities are never fuzzy-merged.
