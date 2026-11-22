The CLI bundles Payday's eight default entities from `schema/payday/`, the optional entities from `schema/past/`, `schema/notify/` and `schema/entitle/`, and their framework options and wire types from `proto/`. The files are copied from the same Apache-2.0-licensed revision. [Upstream Payday source](https://github.com/lesomnus/payday/tree/53acc135cc084adafda524e882f19275c0d0cc96)

During generation the CLI rewrites the entity package and local imports for the consuming application. The app's Payday options select the optional entities and their domain numbers.
