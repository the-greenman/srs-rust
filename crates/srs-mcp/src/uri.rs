//! Re-export of the transport-independent `srs://` resource URI contract.
//!
//! Native rmcp and browser JSON-RPC adapters must parse identical resource
//! addresses. The implementation lives in `srs-mcp-core`.

pub use srs_mcp_core::uri::*;
