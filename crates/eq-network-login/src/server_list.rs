use crate::login::AppOp;

/// One world advertised by an EQEmu-compatible login server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEntry {
    /// World server address as advertised by the login server.
    pub ip: String,
    /// Stable list identifier used by selection requests.
    pub list_id: u32,
    /// Runtime world identifier.
    pub runtime_id: u32,
    /// Display name shown to clients.
    pub name: String,
    /// Advertised language label.
    pub language: String,
    /// Advertised region label.
    pub region: String,
    /// Raw availability status.
    pub status: u32,
    /// Advertised online player count.
    pub player_count: u32,
    /// Original encoded entry used for lossless filtering and rebuilding.
    pub raw: Vec<u8>,
}

#[must_use]
/// Parse a server-list response and retain its opaque 16-byte header.
pub fn parse_server_list(app_payload: &[u8]) -> Option<(Vec<ServerEntry>, [u8; 16])> {
    if app_payload.len() < 20 {
        return None;
    }
    let data = &app_payload[2..];
    let mut header = [0u8; 16];
    header.copy_from_slice(&data[..16]);
    let count = u32::from_le_bytes(data[16..20].try_into().ok()?) as usize;
    let mut pos = 20;
    let mut servers = Vec::new();
    while pos < data.len() && servers.len() < count {
        let start = pos;
        let Some(ip) = read_cstr(data, &mut pos) else {
            break;
        };
        let Some(list_id) = read_u32_le(data, &mut pos) else {
            break;
        };
        let Some(runtime_id) = read_u32_le(data, &mut pos) else {
            break;
        };
        let Some(name) = read_cstr(data, &mut pos) else {
            break;
        };
        let Some(language) = read_cstr(data, &mut pos) else {
            break;
        };
        let Some(region) = read_cstr(data, &mut pos) else {
            break;
        };
        let Some(status) = read_u32_le(data, &mut pos) else {
            break;
        };
        let Some(player_count) = read_u32_le(data, &mut pos) else {
            break;
        };
        servers.push(ServerEntry {
            ip,
            list_id,
            runtime_id,
            name,
            language,
            region,
            status,
            player_count,
            raw: data[start..pos].to_vec(),
        });
    }
    Some((servers, header))
}

fn read_u32_le(data: &[u8], pos: &mut usize) -> Option<u32> {
    let bytes = data.get(*pos..*pos + 4)?;
    *pos += 4;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn read_cstr(data: &[u8], pos: &mut usize) -> Option<String> {
    let start = *pos;
    let end = data[*pos..].iter().position(|&b| b == 0)? + *pos;
    let s = String::from_utf8_lossy(&data[start..end]).to_string();
    *pos = end + 1;
    Some(s)
}

#[must_use]
/// Encode a login-server list from entries retaining their original wire fields.
///
/// The count saturates only when the input contains more entries than the wire's
/// `u32` count can represent, a collection too large for practical packet use.
pub fn build_server_list_response(servers: &[ServerEntry], header_bytes: &[u8; 16]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(AppOp::ServerListResponse as u16).to_le_bytes());
    out.extend_from_slice(header_bytes);
    let count = u32::try_from(servers.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());
    for s in servers {
        out.extend_from_slice(&s.raw);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_server_list_round_trips() {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"192.0.2.1\0");
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.extend_from_slice(&2u32.to_le_bytes());
        raw.extend_from_slice(b"Example Server\0English\0US\0");
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.extend_from_slice(&42u32.to_le_bytes());
        let server = ServerEntry {
            ip: "192.0.2.1".into(),
            list_id: 1,
            runtime_id: 2,
            name: "Example Server".into(),
            language: "English".into(),
            region: "US".into(),
            status: 1,
            player_count: 42,
            raw,
        };
        let header = [7; 16];
        let encoded = build_server_list_response(&[server], &header);
        let (servers, decoded_header) = parse_server_list(&encoded).expect("synthetic list");
        assert_eq!(decoded_header, header);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Example Server");
    }
}
