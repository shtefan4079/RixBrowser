
#![deny(unsafe_code)]

pub mod cache;
pub mod client;
pub mod decompress;
pub mod dns_tls;
pub mod error;
pub mod resource;
pub mod timing;
pub mod sabr;
pub mod signature_cipher;

pub use cache::{CacheControlDirectives, HttpCache, HttpCacheEntry, HttpCacheMeta};
pub use client::{parse_cookie_header, ConditionalRequest, H3Sender, HttpClient, HttpResponse};
pub use decompress::{
    decompress_body, decompress_body_for_header, encodings_from_header, ContentEncoding,
};
pub use dns_tls::{DnsResolver, NegotiatedProtocol, TlsManager, ALPN_PROTOCOLS};
pub use error::{NetError, Result};
pub use resource::{
    sniff_mime_type, sniff_mime_type_with_name, DiskCache, Resource, ResourceManager,
    ResourcePriority,
};
pub use timing::NavigationTiming;
pub use sabr::{
    build_sabr_request, build_sabr_request_params, parse_media_header, parse_ump, parse_ump_response,
    ClientInfo, MediaHeader, MediaSegment, SabrRequestParams, StreamerContext, UmpPart,
};
pub use signature_cipher::{decipher, decipher_signature, PLAYER_VERSION};
