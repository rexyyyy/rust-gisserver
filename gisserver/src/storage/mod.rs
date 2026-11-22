//! Storage layer: positioned byte-range sources and a decoded-tile cache.

pub mod cache;
pub mod source;

pub use cache::{TileCache, TileKey};
pub use source::{
    read_exact_at, HttpRangeSource, LocalFileRangeSource, RangeSource, S3Config, S3RangeSource,
    source_from_uri, source_from_uri_with_s3,
};
