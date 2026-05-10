use jsonwebtoken::{encode, Header, EncodingKey};
use serde::{Serialize, Deserialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
    iss: String,
    aud: String,
}

fn main() {
    let secret = "my-very-secure-shared-secret-for-jwt";
    let my_claims = Claims {
        sub: "test-client".to_string(),
        exp: (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600) as usize,
        iss: "nexus-balancer".to_string(),
        aud: "api-clients".to_string(),
    };

    let token = encode(
        &Header::default(),
        &my_claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    ).unwrap();

    println!("{}", token);
}
