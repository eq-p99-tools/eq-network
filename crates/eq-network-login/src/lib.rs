//! Pure packet codecs for EverQuest-compatible login servers.
//!
//! DES-CBC exists only for legacy wire compatibility and provides no modern
//! confidentiality or integrity.

/// Legacy DES-CBC helpers required by login-server wire formats.
pub mod crypto;
/// Login codec error types.
pub mod error;
/// Login request, result, and application-opcode codecs.
pub mod login;
/// Login server-list parsing and encoding.
pub mod server_list;

pub use crypto::{des_decrypt, des_encrypt, DesKeyIv, DEFAULT_DES_IV, DEFAULT_DES_KEY};
pub use login::{
    build_combined_ack_then_packet, build_login_accepted_combined, build_login_combined,
    encrypt_login_credentials, is_bad_password_login_result, AppOp, LoginPacket,
    LOGIN_RESULT_FAILURE_STATUS,
};
pub use server_list::{build_server_list_response, parse_server_list, ServerEntry};
