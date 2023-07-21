use base64::{engine::general_purpose::STANDARD as b64, Engine};

pub fn encode(text: String, encoding: String) -> String {
    let text = match encoding.as_str() {
        "b64" => b64.encode(text.as_bytes()),
        _ => text,
    };

    text
}

pub fn decode(text: String, encoding: String) -> String {
    let text = match encoding.as_str() {
        "b64" => {
            String::from_utf8(b64.decode(text.as_bytes()).unwrap_or_default()).unwrap_or_default()
        }
        _ => text,
    };

    text
}