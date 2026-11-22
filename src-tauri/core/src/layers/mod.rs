//! Install manifests (docs/loader-manifests.md): the canonical form, the
//! schema, placeholders and the lint rules of `npm run manifests:check`.

pub mod canonical;
pub mod lint;
pub mod placeholders;
pub mod schema;
