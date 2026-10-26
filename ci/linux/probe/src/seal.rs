//! `seal <p256dh> <auth> <plaintext>` seals `plaintext` with the token's keys, base64url
//! encoded like the FCM data map carries them, and prints the sealed envelope's base64url.
//! It exits with failure when a key does not decode or the plaintext exceeds the bound.

use std::process::ExitCode;

use pushups_sender::{SealError, encrypt};
use pushups_token::{EncryptionKeys, Token, base64url};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, p256dh, auth, plaintext] = args.as_slice() else {
        eprintln!("usage: seal <p256dh> <auth> <plaintext>");
        return ExitCode::from(2);
    };
    let (p256dh, auth) = (
        base64url::decode(p256dh).and_then(|bytes| <[u8; 65]>::try_from(bytes).ok()),
        base64url::decode(auth).and_then(|bytes| <[u8; 16]>::try_from(bytes).ok()),
    );
    let (Some(p256dh), Some(auth)) = (p256dh, auth) else {
        eprintln!("the keys are not unpadded base64url of 65 and 16 bytes");
        return ExitCode::from(2);
    };
    let token = Token::Fcm {
        token: String::new(),
        keys: Some(EncryptionKeys { p256dh, auth }),
    };
    match encrypt(&token, plaintext.as_bytes()) {
        Ok(sealed) => {
            println!("{}", sealed.to_base64url());
            ExitCode::SUCCESS
        }
        Err(SealError::TooLarge { .. }) => {
            eprintln!("the plaintext exceeds the sealed bound");
            ExitCode::from(2)
        }
        Err(error) => {
            eprintln!("the seal failed: {error}");
            ExitCode::FAILURE
        }
    }
}
