//! Print a development HS256 token for the sync server.
//!
//! ```text
//! COLLAB_JWT_SECRET=dev-secret cargo run -p wordcraft-collab-server --example dev-token -- editor alice
//! ```
//!
//! Production tokens are issued by the cabinet with an ES256 key (ADR-0008);
//! this example exists only for local runs and manual checks.

use std::process::ExitCode;

use jsonwebtoken::{EncodingKey, Header};
use wordcraft_collab_server::Claims;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let role = args.next().unwrap_or_else(|| "editor".to_string());
    let subject = args.next().unwrap_or_else(|| "alice".to_string());
    let doc = args.next().unwrap_or_else(|| "doc-1".to_string());
    let Some(secret) = std::env::var("COLLAB_JWT_SECRET").ok() else {
        eprintln!("set COLLAB_JWT_SECRET to the secret the server was started with");
        return ExitCode::from(2);
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let claims =
        Claims { sub: subject, aud: "collab".into(), doc, app: "wordcraft".into(), role, sid: "dev-session".into(), iat: now, exp: now + 3600 };
    match jsonwebtoken::encode(&Header::new(jsonwebtoken::Algorithm::HS256), &claims, &EncodingKey::from_secret(secret.as_bytes())) {
        Ok(token) => {
            println!("{token}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("cannot encode a token: {err}");
            ExitCode::FAILURE
        }
    }
}
