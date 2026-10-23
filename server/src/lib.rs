pub mod domain;
pub mod extract;
pub mod zetaris;

pub use domain::{analyze, Decision, Doc, ExtractRow, Kind};
pub use extract::{classify, parse_upload, sha256_hex};
