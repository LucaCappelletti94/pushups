//! `send <subscription.json> <vapid-private.pem> <payload>` sends one Web Push with the `web-push`
//! crate and prints the push service's answer. It exits with failure unless the service accepted it.
//!
//! A server error, a rate limit or a network failure is retried for up to a minute, since a public
//! push service such as ntfy.sh answers so now and then.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushError, WebPushMessageBuilder,
};

const RETRY_BOUND: Duration = Duration::from_secs(60);
const RETRY_PAUSE: Duration = Duration::from_secs(3);

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let [_, subscription, pem, payload] = args.as_slice() else {
        eprintln!("usage: send <subscription.json> <vapid-private.pem> <payload>");
        return ExitCode::from(2);
    };
    match send(subscription, pem, payload).await {
        Ok(()) => {
            println!("accepted");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("refused: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn send(
    subscription: &str,
    pem: &str,
    payload: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let subscription: SubscriptionInfo =
        serde_json::from_str(&std::fs::read_to_string(subscription)?)?;
    let client = IsahcWebPushClient::new()?;
    let deadline = Instant::now() + RETRY_BOUND;
    loop {
        let mut signature =
            VapidSignatureBuilder::from_pem(std::fs::File::open(pem)?, &subscription)?;
        signature.add_claim("sub", "mailto:pushups-ci@users.noreply.github.com");
        let mut message = WebPushMessageBuilder::new(&subscription);
        message.set_payload(ContentEncoding::Aes128Gcm, payload.as_bytes());
        message.set_vapid_signature(signature.build()?);
        message.set_ttl(300);
        let pause = match client.send(message.build()?).await {
            Ok(()) => return Ok(()),
            Err(WebPushError::ServerError { retry_after, .. }) => {
                retry_after.unwrap_or(RETRY_PAUSE).min(RETRY_BOUND)
            }
            Err(WebPushError::Io(_) | WebPushError::Unspecified) => RETRY_PAUSE,
            Err(error) => return Err(error.into()),
        };
        if Instant::now() + pause >= deadline {
            return Err("the push service kept answering with a transient error".into());
        }
        println!("transient refusal, retrying in {} s", pause.as_secs());
        tokio::time::sleep(pause).await;
    }
}
